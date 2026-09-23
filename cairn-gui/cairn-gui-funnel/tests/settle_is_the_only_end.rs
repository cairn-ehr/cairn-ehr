//! ⇒ NO SHIPPED CODE DESTRUCTURES A REGISTRATION OUTCOME AND THROWS THE ATTESTATION AWAY.
//!
//! `TokenStore::settle` exists because the natural Rust idiom for the funnel's `register` port is
//! a silent trap. The port hands the `AttestedSearch` back *inside* its error, precisely so
//! `restore` has the value it needs — which makes this compile, and read as tidy:
//!
//! ```ignore
//! let id = live.register(attested, name).await.map_err(|(e, _)| e)?;
//! ```
//!
//! It drops the attestation during **destructuring**, so `#[must_use]` cannot fire (that lint
//! sees unused expression *results*, not discarded pattern fields), and it leaves `in_flight`
//! set. From there every later `take` returns `TokenError::RegistrationInFlight`, `discard`
//! deliberately does not clear the flag, and the clerk reads *"this registration is already being
//! saved — wait for it to finish"* **forever**, with no gesture that recovers. A window reload is
//! the only way out.
//!
//! `settle` makes the correct path the SHORT one. It does not make the wrong one unrepresentable:
//! `commit` and `restore` are still `pub`, because `settle` delegates to them and a caller with a
//! genuinely different shape must still be able to reach them. A borrow-holding guard would have
//! closed it structurally but cannot survive an `.await`, and attaching the latch to
//! `AttestedSearch` via `Drop` was judged a larger change than this slice should make.
//!
//! So the remaining hole is ergonomic, and this is the forward-only guard for it — the same shape
//! `enrolment_is_never_a_write_side_effect.rs` uses one crate over, and for the same reason: the
//! rule is a call-site question, and a call-site question is cheaper to answer by reading the
//! source than by building a rig per caller.
//!
//! ⚠️ **What this guard CANNOT catch, stated so nobody mistakes it for the whole fix.** It sees
//! source, so it sees only the route a caller *writes*. The other route to the same latched store
//! is a `register` future that is **dropped** rather than awaited — window closed mid-write,
//! webview reload, a `select!`, a timeout, a panic through the await. The `AttestedSearch` is then
//! dropped inside the future, `in_flight` stays set, and `settle` is never reached. No source
//! pattern is wrong; the caller simply never runs. That needs a `Drop` on `AttestedSearch`
//! carrying the latch, and is [#669](https://github.com/cairn-ehr/cairn-ehr/issues/669).
//!
//! **Scope: shipped `src/` code in the `cairn-gui` tree only.** Tests legitimately use the idiom —
//! several exist to prove what it costs — so flagging them would make the guard noise. Slice 2c's
//! handler is shipped code, and it is the caller this exists for.
//!
//! Found by the PR #661 review (two reviewers, independently).

use std::path::{Path, PathBuf};

/// Every `.rs` file under a `src/` directory in the `cairn-gui` tree.
///
/// A local walker rather than `cairn-node`'s `tests/common/sources.rs`: that helper lives in the
/// other cargo workspace, and copying twenty lines beats making one tree's test harness a
/// dependency of the other's.
fn shipped_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `file_type()` rather than `is_dir()`: it does not follow symlinks, so a stray link
        // cannot send the walk somewhere unbounded.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let name = entry.file_name();
            // `target` holds build output, and `tests`/`benches` are not shipped — the same skip
            // list, and the same reasoning, as the sibling guard in `cairn-node`.
            if name != "target" && name != "tests" && name != "benches" {
                shipped_sources(&path, out);
            }
        } else if file_type.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Strip a line comment, so the module docs that *quote* the trap are not read as committing it.
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

#[test]
fn shipped_code_settles_a_registration_rather_than_destructuring_past_it() {
    // `cairn-gui-funnel/tests/` → the `cairn-gui` tree root.
    let tree = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cairn-gui-funnel sits inside the cairn-gui tree")
        .to_path_buf();

    let mut files = Vec::new();
    for entry in std::fs::read_dir(&tree)
        .expect("the cairn-gui tree must be readable")
        .flatten()
    {
        let src = entry.path().join("src");
        if src.is_dir() {
            shipped_sources(&src, &mut files);
        }
        // The tab crates nest one level deeper (`cairn-gui-tabs/cairn-gui-tab-*/src`), and a
        // tab is exactly the kind of place a registration handler could grow.
        if entry.path().is_dir() {
            for nested in std::fs::read_dir(entry.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let nested_src = nested.path().join("src");
                if nested_src.is_dir() {
                    shipped_sources(&nested_src, &mut files);
                }
            }
        }
    }
    assert!(
        files.len() > 10,
        "the source sweep collapsed to {} files — a guard that scans nothing passes for the \
         wrong reason",
        files.len()
    );

    let mut offenders = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let code = strip_comment(line);
            // The shape, not the whole idiom: any closure destructuring a two-field error tuple
            // and naming only the first field discards the attestation. `|(e, _)|`, `|(err, _)|`
            // and `|(e, _returned)|` are all the same mistake, so the test is "a tuple pattern
            // whose second binding starts with `_`".
            if code.contains("map_err(|(") && code.contains(", _") {
                offenders.push(format!(
                    "  {}:{} — {}",
                    path.strip_prefix(&tree).unwrap_or(path).display(),
                    n + 1,
                    code.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "shipped code is destructuring a registration outcome and discarding the attested \
         search.\n\nOffending sites:\n{}\n\nUse `TokenStore::settle(outcome)` instead: it \
         commits on `Ok`, restores on `Err`, and returns the `Restored` so the window can tell \
         \"press Register again\" from \"wait for the next search\". Discarding the attestation \
         here leaves `in_flight` set, and the clerk then reads \"already being saved — wait for \
         it to finish\" for the rest of the window's life, with no gesture that recovers \
         (#659). If a caller genuinely needs `commit`/`restore` directly, call them by name — \
         that is a deliberate, reviewable act; this idiom is not.",
        offenders.join("\n")
    );
}
