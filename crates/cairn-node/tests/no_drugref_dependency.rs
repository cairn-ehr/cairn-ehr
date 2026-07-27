//! ADR-0059 decision 4 — honest degradation, proven by construction.
//!
//! A node without drugref must still read, sync, list and reconcile a CODED medication.
//! The strongest possible proof of that is structural: no drugref code exists in the
//! trusted surface this guard scans, so drugref-absent is the ONLY configuration every
//! other test runs under. A mocked absence could drift; this cannot.
//!
//! SCOPE (what this guard actually covers, so a reader can tell coverage from
//! aspiration): every `.sql` under `db/`, every `.rs` under `crates/*/src`, and every
//! `.sql`/`.rs` under `extensions/*` — that last one is the pgrx tree
//! (`extensions/cairn_pgx`), the in-DB floor's OTHER home besides `db/`, and just as
//! load-bearing. Any directory named `target/` or `tests/` is skipped at any depth —
//! build output and test-only code may legitimately NAME drugref in prose.
//!
//! What this guard CANNOT see: a dependency declared in a `Cargo.toml` under an alias
//! (e.g. `drug_db = { package = "drugref-client", … }`) would sail through untouched —
//! manifests are never read here. This is a source-code guard, not a supply-chain audit.
//!
//! When a later slice adds the §9 advisory-tier drugref lookup, this guard must be
//! narrowed deliberately (to the trusted surface — db/ and the floor path), never simply
//! deleted: the load-bearing invariant is that the FLOOR and the PROJECTIONS never depend
//! on a drug database, not that no client code exists anywhere.
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every `.sql`/`.rs` under db/, crates/, and extensions/ — the trusted surface (the
/// in-DB floor plus the Rust code that submits and projects through it). `extensions/`
/// holds the pgrx floor (`extensions/cairn_pgx`) — easy to forget because it is a
/// SEPARATE Cargo/pgrx build from the `crates/` workspace, but it is exactly as
/// load-bearing as `db/`, so a guard that skipped it would be proving less than its own
/// doc comment claims.
fn trusted_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![
        repo_root().join("db"),
        repo_root().join("crates"),
        repo_root().join("extensions"),
    ];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let p = entry.expect("dir entry").path();
            if p.is_dir() {
                // tests/ may legitimately NAME drugref in prose; src/ and db/ may not.
                if p.file_name().is_some_and(|n| n == "target" || n == "tests") {
                    continue;
                }
                stack.push(p);
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("sql") | Some("rs")
            ) {
                out.push(p);
            }
        }
    }
    out
}

/// Blank out (replace with spaces, preserving every newline) every character that
/// lives inside a string literal or a same-line comment, leaving only the residue of
/// *executable code* on each line. This is the structural test for "is this drugref
/// mention a dependency, or just prose/data that happens to say the word" — it
/// recognises the SHAPE that makes text inert (inside quotes, or after a comment
/// marker) rather than hand-enumerating specific phrases. A phrase list rots: it grows
/// every time someone rewords a diagnostic message, and a guard whose exclusion list
/// keeps growing on every prose edit is a guard someone eventually deletes. It also
/// closes a real gap a phrase list can't: a line carrying BOTH an exempt phrase and a
/// genuine call — e.g. `RAISE EXCEPTION 'a drugref moiety id is a UUIDv5' ||
/// drugref_lookup(x)` — still trips this check, because only the QUOTED portion is
/// blanked; `drugref_lookup(x)` survives in the residue.
///
/// Walks the whole file as one character stream, not line-by-line, because a Rust
/// string can span a line break via `\<newline>` continuation (this tree has exactly
/// that shape in an `assert!` message) — resetting "am I inside a string" at each line
/// boundary would misread the continuation line as bare code and false-flag it.
///
/// Language-aware, because SQL and Rust disagree about what a quote means: SQL strings
/// are single-quoted (`'...'`, doubled `''` to escape a literal quote) and comments
/// start `--`; Rust strings are double-quoted (`"..."`, `\"` to escape), `'` is a char
/// literal or a lifetime and never starts a string, and line comments start `//`
/// (covering `//`, `///`, `//!` alike — all three share the two-slash prefix).
fn residue_lines(text: &str, is_rust: bool) -> Vec<String> {
    let str_delim = if is_rust { '"' } else { '\'' };
    let comment: [char; 2] = if is_rust { ['/', '/'] } else { ['-', '-'] };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            if is_rust && c == '\\' && i + 1 < chars.len() {
                // An escape inside a Rust string: `\"`, `\\`, or the `\<newline>` line
                // continuation. Consume both characters as inert so an escaped quote
                // never looks like the string's closing quote, and — critically for
                // the continuation case — `in_string` stays true across the newline.
                out.push(' ');
                out.push(if chars[i + 1] == '\n' { '\n' } else { ' ' });
                i += 2;
                continue;
            }
            if c == str_delim {
                if !is_rust && chars.get(i + 1) == Some(&'\'') {
                    // SQL's doubled-quote escape (`''`) is a literal quote character,
                    // not the string's close.
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    continue;
                }
                in_string = false;
            }
            out.push(if c == '\n' { '\n' } else { ' ' });
            i += 1;
            continue;
        }
        if c == comment[0] && chars.get(i + 1) == Some(&comment[1]) {
            // Comment marker: blank the rest of the line, but stop AT the newline (not
            // past it) so the outer loop's default branch below re-emits it untouched
            // and the char-count-per-line stays 1:1 with the original for `.lines()`.
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if c == str_delim {
            in_string = true;
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.lines().map(|l| l.to_string()).collect()
}

/// A drugref mention survives into a line's residue only when it is neither quoted nor
/// commented — i.e. it is actually part of executable code (a call, an identifier, a
/// bare SQL reference). Returns the ORIGINAL line text (trimmed) for a readable
/// failure message, even though the decision was made on the blanked residue.
fn offending_lines(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).expect("read source");
    let is_rust = path.extension().and_then(|e| e.to_str()) == Some("rs");
    let residue = residue_lines(&text, is_rust);
    text.lines()
        .zip(residue.iter())
        .filter(|(_, residue_line)| residue_line.to_lowercase().contains("drugref"))
        .map(|(raw, _)| raw.trim().to_string())
        .collect()
}

#[test]
fn the_trusted_surface_never_calls_drugref() {
    let mut offenders: Vec<String> = Vec::new();
    for path in trusted_sources() {
        for line in offending_lines(&path) {
            offenders.push(format!("{}: {line}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "the in-DB floor and the projections must never depend on a drug database \
         (ADR-0059 decision 4 — a coded medication reads, syncs and reconciles without \
         drugref). Offenders:\n{}",
        offenders.join("\n")
    );
}
