//! Every suppressed security advisory carries a review date, and that date has not passed.
//!
//! # Why this exists
//!
//! `cargo-deny` lets a known advisory be ignored with an `id` and a `reason`. In 0.19.9 those are
//! the **only** two fields it accepts — there is no `expires`. So an ignore is *permanent by
//! default*: it suppresses a security finding for as long as the file says so, and nothing ever
//! asks whether the reasoning still holds.
//!
//! That is the silent-gate shape this repo keeps finding (#442, #446): a gate that supplies
//! reassurance without re-earning it. The cost is not hypothetical. The one ignore in this repo,
//! RUSTSEC-2024-0429, carried an exit condition — *"remove when Tauri's Linux backend moves to
//! gtk-rs 0.20+"* — that was **unreachable**: the `gtk` crate's terminal release is 0.18.2
//! (2024-12-09), because gtk-rs archived the GTK3 bindings and moved to the separate `gtk4`
//! crate. A reader following that sentence would check for a gtk-rs 0.20 that will never ship,
//! see nothing, and re-defer — forever, silently, with a green check each time.
//!
//! An expiry does not fix a wrong rationale. What it does is force someone to *read* the
//! rationale again, on a date, with the advisory in front of them.
//!
//! # What a failure here means
//!
//! Not that the dependency became more dangerous. It means the suppression has gone unexamined
//! for as long as its author was willing to promise. The fix is to re-check the exit condition
//! and then either drop the ignore or extend the date **deliberately** — a re-decision, not a
//! default.
//!
//! # Why the parse is hand-rolled
//!
//! Reading `deny.toml` with a real TOML parser would mean adding `toml` (plus `toml_edit`,
//! `serde_spanned`, `winnow`) to this tree for a test helper. None of them is in the graph today,
//! and #445 is a recent reminder of what a compile-time dependency can cost. The trade is only
//! acceptable because the parse is made **self-checking**: `[[advisories.ignore]]` is counted as
//! a literal marker, and every counted block must yield an `id`, so an ignore written in a shape
//! this file does not understand fails loudly instead of being skipped. A guard that can quietly
//! examine nothing is the thing being guarded against.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many suppressed advisories the repo carries today (2026-08-21).
///
/// A liveness floor on the *scan*, in the shape `cargo_lockfiles_tracked.rs` established: without
/// it, a scan that found no `deny.toml` at all — a moved file, a renamed config — would pass for
/// the same reason a clean repo passes. Dropping to zero suppressions is good news and should be
/// a conscious edit here.
const IGNORES_TODAY: usize = 1;

/// The repository root, as git reports it.
fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&manifest_dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("run git rev-parse");
    assert!(
        out.status.success(),
        "git rev-parse --show-toplevel failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
}

/// Every `deny.toml` the repository tracks.
fn tracked_deny_configs(repo: &Path) -> Vec<PathBuf> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "*deny.toml"])
        .current_dir(repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("run git ls-files");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| repo.join(s))
        .collect()
}

/// Days since 1970-01-01 for a proleptic-Gregorian date. Howard Hinnant's `days_from_civil`.
///
/// Pure integer arithmetic so the comparison needs no date crate: shifting the year to start in
/// March makes the leap day the last day of the year, which removes every special case.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Today, as days since the epoch.
fn today() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_secs() as i64;
    secs / 86_400
}

/// Parse a `YYYY-MM-DD` marker into days since the epoch.
///
/// Deliberately strict: a malformed date is an error, never a silently-skipped check.
fn parse_review_date(s: &str) -> Result<i64, String> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(format!("`{s}` is not a YYYY-MM-DD date"));
    }
    let num = |p: &str| {
        p.parse::<i64>()
            .map_err(|_| format!("`{s}` is not numeric"))
    };
    let (y, m, d) = (num(parts[0])?, num(parts[1])?, num(parts[2])?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(format!("`{s}` is not a real date"));
    }
    Ok(days_from_civil(y, m, d))
}

/// The `review-by YYYY-MM-DD` marker inside a `reason` string, if present.
fn review_date_in(reason: &str) -> Option<&str> {
    let idx = reason.find("review-by ")? + "review-by ".len();
    let rest = &reason[idx..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// One suppressed advisory, as read from a config file.
struct Ignore {
    file: String,
    id: String,
    reason: String,
}

/// Read the `[[advisories.ignore]]` blocks out of one config.
///
/// The block count is taken from the literal marker and returned alongside the parsed entries, so
/// the caller can assert the two agree rather than trusting that everything was seen.
fn parse_ignores(path: &Path, repo: &Path) -> (Vec<Ignore>, usize) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("unreadable config {}: {e}", path.display()));
    let file = path
        .strip_prefix(repo)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    // cargo-deny also accepts `ignore = [ … ]` directly under `[advisories]`. This repo does not
    // use that form, and rather than half-support it the guard refuses it outright — an
    // unparsed shape must never read as "no suppressions here".
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("ignore") && t.contains('=') && !t.starts_with('#') {
            panic!(
                "{file} uses the `ignore = [...]` array form. This guard only understands the \
                 `[[advisories.ignore]]` table form, which is what carries a `reason` a review \
                 date can live in. Convert it, or teach this test the other shape — do not leave \
                 a suppression it cannot see."
            );
        }
    }

    let marker_count = text.matches("[[advisories.ignore]]").count();
    let mut out = Vec::new();

    for block in text.split("[[advisories.ignore]]").skip(1) {
        // A block ends at the next section header at column 0.
        let block = block.find("\n[").map(|i| &block[..i]).unwrap_or(block);
        let field = |name: &str| -> Option<String> {
            block.lines().map(str::trim).find_map(|l| {
                let rest = l.strip_prefix(name)?.trim_start().strip_prefix('=')?;
                Some(rest.trim().trim_matches('"').to_string())
            })
        };
        if let Some(id) = field("id") {
            out.push(Ignore {
                file: file.clone(),
                id,
                reason: field("reason").unwrap_or_default(),
            });
        }
    }

    (out, marker_count)
}

/// The guard: every suppressed advisory names a review date, and none has passed.
#[test]
fn every_advisory_ignore_carries_an_unexpired_review_date() {
    let repo = repo_root();
    let configs = tracked_deny_configs(&repo);
    assert!(
        !configs.is_empty(),
        "found no tracked deny.toml under {} — the scan is looking in the wrong place and would \
         now pass without checking anything.",
        repo.display()
    );

    let mut ignores = Vec::new();
    for cfg in &configs {
        let (parsed, markers) = parse_ignores(cfg, &repo);
        assert_eq!(
            parsed.len(),
            markers,
            "{} has {markers} `[[advisories.ignore]]` block(s) but {} yielded an `id` — this \
             guard would silently skip the difference.",
            cfg.display(),
            parsed.len()
        );
        ignores.extend(parsed);
    }

    assert!(
        ignores.len() >= IGNORES_TODAY,
        "found {} advisory ignore(s), fewer than the {IGNORES_TODAY} this repo is known to carry \
         — either the scan has gone stale, or a suppression was genuinely removed (good news: \
         lower IGNORES_TODAY in the same commit).",
        ignores.len()
    );

    let now = today();
    let mut problems = Vec::new();

    for ig in &ignores {
        match review_date_in(&ig.reason).map(parse_review_date) {
            None => problems.push(format!(
                "  {} — {} carries no `review-by YYYY-MM-DD` in its `reason`.\n    cargo-deny has \
                 no `expires` field, so without one this suppression is permanent by default.",
                ig.file, ig.id
            )),
            Some(Err(e)) => problems.push(format!("  {} — {}: {e}", ig.file, ig.id)),
            Some(Ok(day)) if day < now => problems.push(format!(
                "  {} — {} is due for review ({} days ago).\n    Re-read the rationale above it \
                 with the advisory in hand, then either drop the ignore or extend the date \
                 deliberately.",
                ig.file,
                ig.id,
                now - day
            )),
            Some(Ok(_)) => {}
        }
    }

    assert!(
        problems.is_empty(),
        "suppressed security advisories need review:\n{}",
        problems.join("\n")
    );
}

/// The date arithmetic, against values computed independently (Python's `datetime`).
///
/// `days_from_civil` is the one piece of real logic here and the easiest to get subtly wrong —
/// leap years, the century rule, and negative (pre-epoch) dates all have their own edge. A guard
/// whose comparison is off by a day would expire suppressions early or late without ever saying
/// so, which is the failure this whole file exists to prevent, one level down.
#[test]
fn epoch_day_arithmetic_matches_the_civil_calendar() {
    let cases = [
        (1970, 1, 1, 0),       // the epoch itself
        (1970, 1, 2, 1),       // the day after
        (1969, 12, 31, -1),    // before the epoch — the negative branch
        (2000, 3, 1, 11017),   // the day after a century leap day (2000 IS a leap year)
        (2024, 2, 29, 19782),  // an ordinary leap day
        (2026, 8, 21, 20686),  // the day this guard was written
        (2026, 11, 21, 20778), // the review date it currently enforces
        (1900, 1, 1, -25567),  // 1900 is NOT a leap year — the century rule
        (2100, 1, 1, 47482),   // nor is 2100
    ];
    for (y, m, d, want) in cases {
        assert_eq!(
            days_from_civil(y, m, d),
            want,
            "days_from_civil({y}, {m}, {d})"
        );
    }
}

/// Parsing is strict, because a date this file cannot read must never read as "no date needed".
#[test]
fn review_dates_parse_strictly() {
    assert_eq!(parse_review_date("2026-11-21"), Ok(20778));

    for bad in [
        "2026-13-21", // month out of range
        "2026-11-45", // day out of range
        "2026-1-21",  // unpadded month
        "26-11-21",   // two-digit year
        "2026/11/21", // wrong separator
        "notadate",
        "",
    ] {
        assert!(
            parse_review_date(bad).is_err(),
            "`{bad}` should not parse as a review date"
        );
    }

    // The marker is extracted from free prose, so it must stop at the first non-date character
    // rather than swallowing the rest of the sentence.
    assert_eq!(
        review_date_in("…tracked in issue #389. review-by 2026-11-21"),
        Some("2026-11-21")
    );
    assert_eq!(
        review_date_in("review-by 2026-11-21, then drop it"),
        Some("2026-11-21")
    );
    assert_eq!(review_date_in("no marker here"), None);
}
