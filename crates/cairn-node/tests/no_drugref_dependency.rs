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
//! WHAT COUNTS AS AN OFFENDER: a "drugref" mention that is neither (a) inside a
//! comment, (b) inside a recognised DIAGNOSTIC-MESSAGE string (the argument of
//! `RAISE EXCEPTION`/`RAISE NOTICE`/`RAISE WARNING` in SQL, or `assert!`/`assert_eq!`/
//! `panic!`/`format!`/`eprintln!`/`.expect(` and kin in Rust — see `residue_lines`'s doc
//! for the exact list), (c) inside the ONE statement that seeds the ADR-0059
//! coding-system registry (`db/041`'s `INSERT INTO medication_coding_system`, where the
//! `note` column legitimately narrates drugref in prose next to the tokens it defines),
//! or (d) itself exactly one of the three registered coding-system tokens
//! (`drugref-moiety` / `drugref-clinical-drug` / `drugref-product`) — those are DATA,
//! not a dependency, wherever a test fixture or the registry names them. A drugref
//! mention inside any OTHER string — a URL, a connection string, a shelled-out command
//! — is NOT exempt and fails the guard, same as it would in bare code.
//!
//! WHAT THIS GUARD CANNOT SEE: a dependency declared in a `Cargo.toml` under an alias
//! (e.g. `drug_db = { package = "drugref-client", … }`) — manifests are never read
//! here. Nor does it do full Rust/SQL lexing: raw strings (`r"…"`, `r#"…"#`), `/* */`
//! block comments, and multi-line string forms other than the one shape actually in
//! this tree (a Rust string continued across a line break via `\<newline>`, which IS
//! handled) are not specially recognised. This is a source-code guard, not a
//! supply-chain audit or a full parser — "crude but sufficient" for the shapes this
//! tree actually contains, re-verified each time this file changes (see the non-vacuity
//! evidence in the slice's task report).
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

/// The three tokens ADR-0059 decision 2 registers in `medication_coding_system` — DATA
/// naming the drugref composition-tree levels, not a dependency on drugref itself. A
/// string literal whose content is EXACTLY one of these (checked at the exact-length
/// boundary, so `"drugref-moiety-extended"` would NOT match) is exempt wherever it
/// appears — a test fixture asserting on the anchor value, not just the registry seed
/// row itself, legitimately spells the token out.
const DATA_TOKENS: [&str; 3] = ["drugref-moiety", "drugref-clinical-drug", "drugref-product"];

/// True iff `chars[i..]` begins with `needle`. SQL keywords are matched
/// case-insensitively (SQL itself is); Rust macro/method names and the data tokens are
/// matched case-sensitively (their spelling is fixed by the code, not a human typing
/// SQL).
fn starts_with_at(chars: &[char], i: usize, needle: &str, case_insensitive: bool) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    if i + needle.len() > chars.len() {
        return false;
    }
    chars[i..i + needle.len()]
        .iter()
        .zip(needle.iter())
        .all(|(&a, &b)| {
            if case_insensitive {
                a.eq_ignore_ascii_case(&b)
            } else {
                a == b
            }
        })
}

/// SQL constructs whose quoted argument is a human-readable message or seed data, not a
/// call: `RAISE EXCEPTION`/`NOTICE`/`WARNING` (the message argument explains a rule, it
/// doesn't invoke one), and the ADR-0059 registry seed itself — db/041's ONE
/// `INSERT INTO medication_coding_system` statement, the sole place the three
/// coding-system tokens are DEFINED, whose `note` column narrates them in prose in the
/// same row. Each opens an EXEMPT SPAN that runs to the next unquoted `;` — the
/// statement terminator both constructs share.
const SQL_SPAN_TRIGGERS: [&str; 4] = [
    "RAISE EXCEPTION",
    "RAISE NOTICE",
    "RAISE WARNING",
    "INSERT INTO medication_coding_system",
];

/// Rust macros/calls whose (typically first) string argument is a diagnostic message,
/// not a call target: assertion and panic macros, the format-string family, and the
/// `.expect(` method (leading `.` required so this only matches the method call, not an
/// identifier that merely contains "expect"). Each opens an EXEMPT SPAN tracked by
/// paren depth from its own opening paren (already part of the matched text) back to 0
/// — so a nested call inside the same argument list (e.g. `panic!(format!(...))`)
/// correctly keeps the span open until BOTH close.
const RUST_SPAN_TRIGGERS: [&str; 13] = [
    "assert!(",
    "assert_eq!(",
    "assert_ne!(",
    "debug_assert!(",
    "debug_assert_eq!(",
    "panic!(",
    "unreachable!(",
    "format!(",
    "eprintln!(",
    "println!(",
    "write!(",
    "writeln!(",
    ".expect(",
];

/// Blank out (replace with spaces, preserving every newline so line numbers/splitting
/// stay aligned) the content of every EXEMPT string literal and every same-line
/// comment, leaving everything else — including a NON-exempt string's real content —
/// untouched in the residue. This is the structural test for "is this drugref mention a
/// dependency, or just prose/data that happens to say the word": unlike blanking every
/// string unconditionally (which would also hide a URL or connection string embedded in
/// ordinary code — the exact hole a prior version of this guard had), only a string
/// that is a recognised diagnostic message, the registry seed, or one of the three data
/// tokens ever gets blanked. Everything else — a `let url = "https://…drugref…"` sitting
/// in plain code — survives into the residue and can still trip the guard.
///
/// Walks the whole file as one character stream, not line-by-line, because a Rust
/// string can span a line break via `\<newline>` continuation (this tree has exactly
/// that shape in an `assert!` message) and a diagnostic span can span several lines
/// before its terminator — resetting state at each line boundary would misread a
/// continuation line as something it isn't.
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
    // Fixed once per string, at its opening delimiter — whether THIS string's content
    // gets blanked. Doesn't change mid-string even if surrounding span state would
    // later flip, which can't happen anyway (string content can't itself contain an
    // unescaped trigger/paren/semicolon that the scanner would act on, since the whole
    // in_string branch below skips span/trigger logic entirely).
    let mut blank_this_string = false;
    // SQL: are we inside a RAISE .../INSERT-registry-seed statement right now? Simple
    // boolean, not a counter — these statements don't nest in this tree.
    let mut sql_span_active = false;
    // Rust: how many diagnostic-macro/`.expect(` parens are currently open. >0 means
    // "inside a diagnostic argument list"; tracks nested calls via generic paren
    // counting once a span has opened.
    let mut rust_span_depth: i32 = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        if in_string {
            if is_rust && c == '\\' && i + 1 < chars.len() {
                // An escape inside a Rust string: `\"`, `\\`, or the `\<newline>` line
                // continuation. Consume both characters together so an escaped quote
                // never looks like the string's close, and — critically for the
                // continuation case — `in_string` (and `blank_this_string`) carry
                // across the newline unchanged.
                out.push(if blank_this_string { ' ' } else { c });
                let escaped = chars[i + 1];
                out.push(if escaped == '\n' {
                    '\n'
                } else if blank_this_string {
                    ' '
                } else {
                    escaped
                });
                i += 2;
                continue;
            }
            if c == str_delim {
                if !is_rust && chars.get(i + 1) == Some(&'\'') {
                    // SQL's doubled-quote escape (`''`) is a literal quote character,
                    // not the string's close.
                    out.push(if blank_this_string { ' ' } else { c });
                    out.push(if blank_this_string { ' ' } else { '\'' });
                    i += 2;
                    continue;
                }
                in_string = false;
                out.push(' '); // the closing delimiter itself never carries content
                i += 1;
                continue;
            }
            out.push(if c == '\n' {
                '\n'
            } else if blank_this_string {
                ' '
            } else {
                c
            });
            i += 1;
            continue;
        }

        // Not in a string: does an exempt-span trigger start here?
        if is_rust {
            if let Some(&matched) = RUST_SPAN_TRIGGERS
                .iter()
                .find(|t| starts_with_at(&chars, i, t, false))
            {
                rust_span_depth += 1;
                for ch in matched.chars() {
                    out.push(ch);
                    i += 1;
                }
                continue;
            }
            if rust_span_depth > 0 {
                if c == '(' {
                    rust_span_depth += 1;
                } else if c == ')' {
                    rust_span_depth = (rust_span_depth - 1).max(0);
                }
            }
        } else {
            if let Some(&matched) = SQL_SPAN_TRIGGERS
                .iter()
                .find(|t| starts_with_at(&chars, i, t, true))
            {
                sql_span_active = true;
                for ch in matched.chars() {
                    out.push(ch);
                    i += 1;
                }
                continue;
            }
            if sql_span_active && c == ';' {
                sql_span_active = false;
            }
        }

        // Comment marker: blank the rest of the line (stop AT the newline so the
        // default branch below re-emits it, keeping char-count-per-line 1:1 with the
        // original for `.lines()`).
        if c == comment[0] && chars.get(i + 1) == Some(&comment[1]) {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }

        if c == str_delim {
            in_string = true;
            // Exact-token check: does this string's content, independent of any span,
            // spell exactly one of the three registered coding-system tokens? The
            // length-bounded check (immediately followed by the closing delimiter)
            // means a LONGER string that merely starts with a token does not match.
            let exact_token = DATA_TOKENS.iter().any(|t| {
                starts_with_at(&chars, i + 1, t, false)
                    && chars.get(i + 1 + t.chars().count()) == Some(&str_delim)
            });
            blank_this_string = exact_token
                || if is_rust {
                    rust_span_depth > 0
                } else {
                    sql_span_active
                };
            out.push(' ');
            i += 1;
            continue;
        }

        out.push(c);
        i += 1;
    }
    out.lines().map(|l| l.to_string()).collect()
}

/// A drugref mention survives into a line's residue only when it is neither quoted-and-
/// exempt nor commented — i.e. it is really executable code, or a string this guard has
/// no reason to trust. Returns the ORIGINAL line text (trimmed) for a readable failure
/// message, even though the decision was made on the blanked residue.
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
