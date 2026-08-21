//! ADR-0059 decision 4 — honest degradation, proven by construction.
//!
//! A node without drugref must still read, sync, list and reconcile a CODED medication.
//! The strongest possible proof of that is structural: no drugref code exists in the
//! trusted surface this guard scans, so drugref-absent is the ONLY configuration every
//! other test runs under. A mocked absence could drift; this cannot.
//!
//! SCOPE (what this guard actually covers, so a reader can tell coverage from
//! aspiration): every `.sql`/`.rs` anywhere under `db/`, `crates/` and `extensions/` —
//! NOT merely `crates/*/src`, so a `build.rs`, a `benches/` harness or an `examples/`
//! binary is scanned too. `extensions/` is the pgrx tree (`extensions/cairn_pgx`), the
//! in-DB floor's OTHER home besides `db/`, and just as load-bearing. Any directory named
//! `target/` or `tests/` is skipped at any depth — build output and test-only code may
//! legitimately NAME drugref in prose (this file itself is the obvious case). For the
//! SAME reason, a `#[cfg(test)]` module inside a `src/` file is skipped too: it is not
//! compiled into the shipped artifact at all, so a unit test asserting on a rendered twin
//! string (`"coded as atorvastatin [drugref-moiety]"`) is exactly the prose case the
//! `tests/` skip already allows — slice 6a simply never had one, because all of its
//! drugref-touching tests lived under `tests/`. See `strip_rust_cfg_test_tail` for the
//! stated limitation of how that region is found.
//!
//! WHAT COUNTS AS AN OFFENDER: a "drugref" mention that is neither (a) inside a
//! comment, (b) inside a recognised DIAGNOSTIC-MESSAGE call's argument list (SQL
//! `RAISE EXCEPTION`/`RAISE NOTICE`/`RAISE WARNING`, or Rust `assert!`/`debug_assert!`/
//! `panic!`/`unreachable!`/`format!`/`eprintln!`/`println!`/`write!`/`writeln!`/
//! `bail!`/`ensure!`/`anyhow!`/`.expect(` — see `RUST_SPAN_TRIGGERS`'s doc for exactly
//! which macros and why `assert_eq!`/`assert_ne!`/`debug_assert_eq!` are deliberately
//! NOT in this list), (c) inside the ONE statement that seeds the ADR-0059 coding-system
//! registry (`db/041`'s `INSERT INTO medication_coding_system`, where the `note` column
//! legitimately narrates drugref in prose next to the tokens it defines), or (d) itself
//! exactly one of the three registered coding-system tokens (`drugref-moiety` /
//! `drugref-clinical-drug` / `drugref-product`) — those are DATA, not a dependency,
//! wherever a test fixture or the registry names them. A drugref mention inside any
//! OTHER string — a URL, a connection string, a shelled-out command, an `assert_eq!`'s
//! compared value — is NOT exempt and fails the guard, same as it would in bare code.
//!
//! KNOWN LIMITATION, stated plainly rather than chased further (see the round-3 task
//! report for why the natural next tightening — "blank only the message argument, not
//! the whole call" — was NOT attempted): this guard is a line-oriented character
//! scanner, not a lexer or a parser. For the diagnostic-message macros in (b) above, it
//! exempts the macro's ENTIRE argument list, not just the message argument — so a
//! drugref reference placed in a NON-message argument of one of those calls (e.g.
//! `println!("Fetching {}", "https://api.drugref.org/lookup")`, where the URL is the
//! second, non-message argument) is masked, not caught. It is likewise defeated by a
//! raw string (`r"…"`) or a `/* */` block comment, neither of which it recognises. This
//! guard is a REGRESSION NET for an *accidental* dependency, not a defence against
//! deliberate concealment — the actual load-bearing invariant is enforced by there
//! being no drugref client anywhere in this tree at all, which this guard corroborates
//! by construction; it does not itself prevent a determined author from hiding one.
//!
//! WHAT THIS GUARD CANNOT SEE (mechanically, beyond the limitation above): a dependency
//! declared in a `Cargo.toml` under an alias (e.g. `drug_db = { package =
//! "drugref-client", … }`) — manifests are never read here. This is a source-code
//! guard, not a supply-chain audit.
//!
//! When a later slice adds the §9 advisory-tier drugref lookup, this guard must be
//! narrowed deliberately (to the trusted surface — db/ and the floor path), never simply
//! deleted: the load-bearing invariant is that the FLOOR and the PROJECTIONS never depend
//! on a drug database, not that no client code exists anywhere.
//! The recursive walk is the SHARED one (#452). This file used to carry its own, built on
//! `path.is_dir()` — which FOLLOWS symlinks, so a symlink to an ancestor would have made the
//! walk unbounded. See `tests/common/sources.rs` for why that fix belongs in one place.
#[path = "common/sources.rs"]
mod sources;

use sources::{read_source, repo_root};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Every `.sql`/`.rs` under db/, crates/, and extensions/ — the trusted surface (the
/// in-DB floor plus the Rust code that submits and projects through it). `extensions/`
/// holds the pgrx floor (`extensions/cairn_pgx`) — easy to forget because it is a
/// SEPARATE Cargo/pgrx build from the `crates/` workspace, but it is exactly as
/// load-bearing as `db/`, so a guard that skipped it would be proving less than its own
/// doc comment claims.
///
/// `tests` joins `target` in the skip list because a test may legitimately NAME drugref in
/// prose or in a fixture; `src/` and `db/` may not.
fn trusted_sources() -> Vec<PathBuf> {
    let root = repo_root();
    sources::source_files(
        &[
            root.join("db"),
            root.join("crates"),
            root.join("extensions"),
        ],
        &["target", "tests"],
        &["sql", "rs"],
    )
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

/// Rust macros/calls whose string argument is a diagnostic message, not a call target:
/// plain assertion/panic macros, the format-string family, and the `.expect(` method
/// (leading `.` required so this only matches the method call, not an identifier that
/// merely contains "expect"). Each opens an EXEMPT SPAN tracked by paren depth from its
/// own opening paren (already part of the matched text) back to 0 — so a nested call
/// inside the same argument list (e.g. `panic!(format!(...))`) correctly keeps the span
/// open until BOTH close.
///
/// DELIBERATELY EXCLUDED: `assert_eq!`/`assert_ne!`/`debug_assert_eq!`. Their leading
/// arguments are the two COMPARED VALUES, not a message — a drugref mention there is
/// exactly the kind of reference this guard should see, not exempt (this is why the
/// span blanks the WHOLE argument list rather than just a "first string": narrowing
/// that further, to only the true message argument, would need per-macro
/// argument-position awareness this scanner doesn't have — see the module doc's KNOWN
/// LIMITATION for the resulting gap on the macros kept below, e.g. `assert!`, whose
/// message is its SECOND argument and whose first argument can itself embed an
/// unrelated string, so "blank only the first string" would target the wrong one).
///
/// The `anyhow` family (`bail!`/`ensure!`/`anyhow!`) is in the list because it is how
/// THIS codebase writes a user-facing error message — `coding_from_parts` in
/// `medication/assert.rs` is two files away. Omitting them would have made the first
/// error text to legitimately name drugref ("register it in medication_coding_system,
/// not in drugref") fail the guard spuriously. Each is matched WITHOUT a path prefix, so
/// the bare and `anyhow::`-qualified spellings both hit the same trigger.
const RUST_SPAN_TRIGGERS: [&str; 13] = [
    "assert!(",
    "debug_assert!(",
    "panic!(",
    "unreachable!(",
    "format!(",
    "eprintln!(",
    "println!(",
    "write!(",
    "writeln!(",
    "bail!(",
    "ensure!(",
    "anyhow!(",
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

/// Blank out an in-`src` `#[cfg(test)] mod` region, for the same reason whole `tests/`
/// directories are skipped: it is test-only code, never compiled into the shipped
/// artifact, and may legitimately NAME drugref. A unit test asserting the exact rendered
/// twin `"coded as atorvastatin [drugref-moiety]"` is not a dependency on a drug
/// database — it is a fixture spelling out a registered token inside a longer string,
/// which the `DATA_TOKENS` exemption cannot cover because that requires the literal to be
/// the token EXACTLY.
///
/// The trigger is `#[cfg(test)]` IMMEDIATELY BEFORE A MODULE, not `#[cfg(test)]` alone.
/// That attribute is equally legal on a single item — `#[cfg(test)] use foo::bar;` near the
/// top of a file is ordinary Rust — and treating it as the start of the tail would blank
/// the whole file below it, silently unscanning every line of production code that
/// followed. Nothing in the tree spells it that way today (every `#[cfg(test)]` under
/// `crates/*/src` introduces a `mod`), which is exactly why the widening would have gone
/// unnoticed. Intervening blank lines and further attributes are skipped, so the
/// conventional `#[cfg(test)]` + `mod tests {` still matches however it is spaced.
///
/// STATED LIMITATION, in the same spirit as this file's other honest limits: the region is
/// taken as the module's declaration → END OF FILE, not a brace-matched span. This guard is
/// a line-oriented scanner, not a parser, and Rust convention (which this repo follows
/// everywhere) puts the test module last. Production code placed AFTER a `#[cfg(test)] mod`
/// would therefore go unscanned — a real gap, but a gap in a REGRESSION NET for an
/// accidental dependency, not in a defence against deliberate concealment, which this
/// guard has never claimed to be. Lines are blanked rather than dropped so the residue
/// stays aligned with the original line numbering.
fn strip_rust_cfg_test_tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();

    /// The first line at or after `from` that is not blank and not another attribute —
    /// i.e. the item the `#[cfg(test)]` actually applies to.
    fn item_line(lines: &[&str], from: usize) -> Option<usize> {
        (from..lines.len()).find(|&i| {
            let t = lines[i].trim_start();
            !t.is_empty() && !t.starts_with('#')
        })
    }

    // Where the test tail begins, if anywhere: the first `#[cfg(test)]` whose item is a
    // module declaration.
    let tail_start = lines.iter().enumerate().find_map(|(i, line)| {
        if !line.trim_start().starts_with("#[cfg(test)]") {
            return None;
        }
        let item = item_line(&lines, i + 1)?;
        let t = lines[item].trim_start();
        (t.starts_with("mod ") || t.starts_with("pub mod ") || t.starts_with("pub(crate) mod "))
            .then_some(i)
    });

    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        if tail_start.is_none_or(|start| i < start) {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// A drugref mention survives into a line's residue only when it is neither quoted-and-
/// exempt nor commented — i.e. it is really executable code, or a string this guard has
/// no reason to trust. Returns the ORIGINAL line text (trimmed) for a readable failure
/// message, even though the decision was made on the blanked residue.
fn offending_lines(path: &Path) -> Vec<String> {
    let text = read_source(path);
    let is_rust = path.extension().and_then(|e| e.to_str()) == Some("rs");
    let scanned: Cow<'_, str> = if is_rust {
        Cow::Owned(strip_rust_cfg_test_tail(&text))
    } else {
        Cow::Borrowed(&text)
    };
    let residue = residue_lines(&scanned, is_rust);
    text.lines()
        .zip(residue.iter())
        .filter(|(_, residue_line)| residue_line.to_lowercase().contains("drugref"))
        .map(|(raw, _)| raw.trim().to_string())
        .collect()
}

/// Unit-pin the scanner's exemption rule itself, not just its verdict on today's tree.
/// `the_trusted_surface_never_calls_drugref` below passes trivially while the tree is
/// clean, so it cannot tell a CORRECT scanner from one that exempts (or misses)
/// everything — these cases fix both edges of the rule.
///
/// The `bail!`/`ensure!`/`anyhow!` cases are the ones that motivated this: they are how
/// THIS codebase actually writes an error message (see `coding_from_parts` in
/// `medication/assert.rs`), yet the trigger list originally carried only the std
/// assertion/format macros — so the first error text to legitimately name drugref would
/// have failed the guard spuriously.
/// Pin the `#[cfg(test)]` exemption at BOTH edges: a drugref mention below the marker is
/// exempt (test-only code, never shipped), one above it is not. Without this, a future
/// edit could widen the stripper into "skip the whole file" and nothing would notice.
#[test]
fn the_scanner_exempts_cfg_test_modules_but_not_the_code_above_them() {
    let src = concat!(
        "pub fn ship() { let url = \"https://drugref.example/lookup\"; }\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    #[test]\n",
        "    fn t() { assert_eq!(s, \"coded as x [drugref-moiety]\"); }\n",
        "}\n",
    );
    let residue = residue_lines(&strip_rust_cfg_test_tail(src), true).join("\n");
    assert!(
        residue.to_lowercase().contains("drugref"),
        "production code ABOVE the cfg(test) marker must still be scanned:\n{residue}"
    );
    assert_eq!(
        residue.to_lowercase().matches("drugref").count(),
        1,
        "the cfg(test) tail must be blanked, leaving only the production mention:\n{residue}"
    );
    // And the blanking must preserve line alignment, or the failure message would name
    // the wrong line.
    assert_eq!(
        strip_rust_cfg_test_tail(src).lines().count(),
        src.lines().count(),
        "blanking must keep one output line per input line"
    );
}

/// The exemption is for a `#[cfg(test)] mod`, not for the attribute on its own. On a single
/// item — `#[cfg(test)] use …`, ordinary Rust — treating it as the start of the tail would
/// blank every production line below it, silently unscanning the rest of the file. Nothing
/// in the tree spells it that way today, which is precisely why nothing would have noticed.
#[test]
fn a_cfg_test_attribute_on_a_non_module_item_does_not_blank_the_file() {
    let src = concat!(
        "#[cfg(test)]\n",
        "use std::collections::HashMap;\n",
        "pub fn ship() { let url = \"https://drugref.example/lookup\"; }\n",
    );
    let residue = residue_lines(&strip_rust_cfg_test_tail(src), true).join("\n");
    assert!(
        residue.to_lowercase().contains("drugref"),
        "production code below a non-module cfg(test) item must still be scanned:\n{residue}"
    );
}

/// The conventional spelling still matches however it is spaced: a blank line, a doc
/// comment's sibling attributes, or `pub(crate) mod` must not defeat the exemption.
#[test]
fn the_cfg_test_module_exemption_tolerates_spacing_and_visibility() {
    for module_line in ["mod tests {", "pub mod tests {", "pub(crate) mod tests {"] {
        let src = format!(
            "pub fn ship() {{}}\n#[cfg(test)]\n\n#[allow(clippy::all)]\n{module_line}\n    \
             fn t() {{ assert_eq!(s, \"coded as x [drugref-moiety]\"); }}\n}}\n"
        );
        let residue = residue_lines(&strip_rust_cfg_test_tail(&src), true).join("\n");
        assert!(
            !residue.to_lowercase().contains("drugref"),
            "`{module_line}` must still be recognised as the test module:\n{residue}"
        );
    }
}

#[test]
fn the_scanner_exempts_diagnostic_messages_but_not_ordinary_strings() {
    // Exempt: a drugref mention inside an error/diagnostic message, in each of the
    // anyhow forms (both bare and path-qualified — the scanner matches the macro name
    // wherever it starts, so `anyhow::bail!` hits the same trigger as `bail!`).
    for exempt in [
        r#"bail!("register it in drugref first");"#,
        r#"anyhow::bail!("register it in drugref first");"#,
        r#"ensure!(ok, "drugref must be reachable");"#,
        r#"anyhow::anyhow!("drugref lookup unavailable")"#,
        r#"panic!("drugref went missing");"#,
        r#"let x = y.expect("drugref must be present");"#,
        r#"// a comment naming drugref"#,
    ] {
        let residue = residue_lines(exempt, true).join("\n");
        assert!(
            !residue.to_lowercase().contains("drugref"),
            "this is a diagnostic message or comment and must be exempt: {exempt}"
        );
    }

    // NOT exempt: a real dependency, however string-shaped. These are the cases the
    // guard exists to catch, and blanket string-blanking would have hidden every one.
    for offender in [
        r#"let url = "https://api.drugref.org/lookup";"#,
        r#"assert_eq!(system, "drugref-moiety-extended");"#,
        r#"Command::new("drugref-cli").spawn();"#,
    ] {
        let residue = residue_lines(offender, true).join("\n");
        assert!(
            residue.to_lowercase().contains("drugref"),
            "a real reference must survive into the residue: {offender}"
        );
    }

    // SQL side: the RAISE message and the registry seed are exempt; a connection string
    // is not.
    assert!(
        !residue_lines("RAISE EXCEPTION 'a drugref moiety id is a UUIDv5';", false)
            .join("\n")
            .to_lowercase()
            .contains("drugref")
    );
    assert!(
        residue_lines("PERFORM dblink('host=drugref.internal');", false)
            .join("\n")
            .to_lowercase()
            .contains("drugref")
    );
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
