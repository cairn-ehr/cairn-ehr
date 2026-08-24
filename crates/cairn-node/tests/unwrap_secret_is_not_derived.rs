//! #495 / ADR-0066 — the guard that keeps identity and custody uncoupled.
//!
//! Deriving the node's X25519 unwrap secret from its Ed25519 signing seed is what made a
//! restored solo node unable to open a single one of its own sealed bodies: ADR-0026
//! deliberately mints a fresh seed on recovery, so the derived secret changed and every
//! inherited `event_dek` row went dark. ADR-0066 broke the derivation.
//!
//! `derive_unwrap_secret` still exists, because a node provisioned before ADR-0066 needs
//! it exactly once to ADOPT its old secret as its first independent key. This guard pins
//! that "exactly once": PRODUCTION sources (`crates/*/src/**`) may call it only from the
//! adoption path. Test sources may call it freely — a test establishing some node unwrap
//! key from a signing key is a fixture, not a coupling.
//!
//! WHEN THIS FAILS: do not raise the number to make it green. Ask whether the new call
//! site re-couples custody to identity. If it does, it is the #495 defect returning.
//!
//! Uses the shared source-inspection walker (`tests/common/sources.rs`, #452) rather
//! than rolling a new one — see that file's header for why a from-scratch walk has
//! twice hidden a real defect (symlink-following, swallowed unreadable entries) behind a
//! guard that still printed green. Pulled in the same way `no_drugref_dependency.rs` and
//! `event_log_row_by_name.rs` do: a `#[path = ...]` leaf module, NOT `mod common;` — this
//! is a pure source-inspection binary with no need for `common/mod.rs`'s DB scaffolding,
//! and staying out of `common/mod.rs` also keeps this file's helper out of
//! `identity_scaffolding_shared.rs`'s derivation, which is scoped to the identity
//! cluster specifically (see `sources.rs`'s own module doc, "Why a leaf module").
//!
//! ## Why a bare substring match is not enough
//!
//! `crates/cairn-event/src/seal.rs` is where `derive_unwrap_secret` is DEFINED, and it
//! also carries the function's own `#[cfg(test)]` fixtures (a real seed-in, secret-out
//! round trip is exactly what its unit tests must exercise). Both are legitimate and
//! neither is the #495 coupling — a module is always allowed to name its own function,
//! and this guard's own policy is "test sources may call it freely" — but a raw
//! `text.contains("derive_unwrap_secret")` cannot tell either apart from a real
//! production call site, and would flag `seal.rs` itself on every run. So a "call"
//! here means: outside any trailing `#[cfg(test)]` module, and not the `fn
//! derive_unwrap_secret(` declaration line itself (same declaration-vs-call idiom
//! `identity_scaffolding_shared.rs`'s `locally_declared` already uses one directory
//! over — a definition can never be written in the shape of a call).

#[path = "common/sources.rs"]
mod sources;

use std::path::Path;

/// True iff `line` (any leading indentation) IS the declaration of
/// `derive_unwrap_secret` — as opposed to a call to it. After stripping any visibility
/// prefix (`pub`, `pub(crate)`, `pub(super)`), a declaration always starts `fn
/// derive_unwrap_secret(`; nothing that merely CALLS the function can start that way —
/// a call is an expression, never itself introduced by `fn`.
fn is_the_declaration_line(line: &str) -> bool {
    let line = line.trim_start();
    let line = match line.strip_prefix("pub") {
        Some(rest) => rest.trim_start_matches(|c| c != ' ').trim_start(),
        None => line,
    };
    line.starts_with("fn derive_unwrap_secret(")
}

/// The line index where a trailing `#[cfg(test)] mod` block begins, if the file has
/// one. Same idiom as `no_drugref_dependency.rs`'s `strip_rust_cfg_test_tail` (that
/// file's doc explains the convention: a `src/` file's unit tests conventionally live
/// in a same-file tail, so production code and test fixtures share one file and must be
/// told apart by more than "which file"). Scoped locally rather than shared, because
/// this guard's need — find the boundary once — is narrower than that file's, which
/// also blanks comments and string literals for an unrelated check.
fn cfg_test_tail_start(lines: &[&str]) -> Option<usize> {
    let item_line = |from: usize| -> Option<usize> {
        (from..lines.len()).find(|&i| {
            let t = lines[i].trim_start();
            !t.is_empty() && !t.starts_with('#')
        })
    };
    lines.iter().enumerate().find_map(|(i, line)| {
        if !line.trim_start().starts_with("#[cfg(test)]") {
            return None;
        }
        let item = item_line(i + 1)?;
        let t = lines[item].trim_start();
        (t.starts_with("mod ") || t.starts_with("pub mod ") || t.starts_with("pub(crate) mod "))
            .then_some(i)
    })
}

/// Does this file's PRODUCTION text — excluding any trailing `#[cfg(test)]` module, and
/// excluding `derive_unwrap_secret`'s own declaration line — contain a real call to it?
/// This is the file-level predicate the guard actually means by "calls
/// `derive_unwrap_secret`": naming it in your own signature, or in your own test
/// fixtures, is not that.
fn calls_derive_unwrap_secret(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let tail_start = cfg_test_tail_start(&lines).unwrap_or(lines.len());
    lines[..tail_start]
        .iter()
        .any(|line| line.contains("derive_unwrap_secret") && !is_the_declaration_line(line))
}

/// Production files permitted to call `derive_unwrap_secret`, with the reason each is
/// allowed. A file NOT on this list calling it is the failure this guard exists for.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/cairn-node/src/keystore.rs",
        "the ADR-0066 adoption migration (`adopt_derived_unwrap_secret`) — the one place a \
         pre-ADR-0066 node re-derives its old secret to keep its existing event_dek rows openable",
    ),
    // The two sites ADR-0066 has not yet reached. Both entries are REMOVED/REWRITTEN by a
    // later task in this same slice; they are listed now so that every commit leaves the
    // suite green (house rule 6), never so that the coupling is tolerated.
    (
        "crates/cairn-node/src/medication/sealed_submit.rs",
        "PRE-ADR-0066 — `ensure_unwrap_key` still derives. REMOVE THIS ENTRY in Task 4, \
         which turns that function into a verification and deletes the derivation",
    ),
    (
        "crates/cairn-sync/src/main.rs",
        "PRE-ADR-0066 — cairn-sync cannot read cairn-node's keystore. REWRITE THIS REASON \
         in Task 5 once the startup divergence check exists",
    ),
];

#[test]
fn only_the_adoption_migration_derives_the_unwrap_secret() {
    let root = sources::repo_root();
    let mut offenders: Vec<String> = Vec::new();

    for path in sources::production_rust_files(&root) {
        let text = sources::read_source(&path);
        if !calls_derive_unwrap_secret(&text) {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        if !ALLOWED.iter().any(|(allowed, _)| *allowed == rel) {
            offenders.push(rel);
        }
    }

    assert!(
        offenders.is_empty(),
        "ADR-0066: `derive_unwrap_secret` is the pre-ADR-0066 adoption path ONLY. These \
         production files call it and are not on the allow-list: {offenders:?}. Obtain the \
         node's unwrap secret with `keystore::load_unwrap_secret` instead — deriving it from \
         the signing seed is the coupling that emptied a restored node's whole clinical \
         record (#495)."
    );

    // Anti-vacuity: the guard must be scanning real files, not an empty set. If the
    // helper's globbing breaks, an empty sweep would pass silently and forever.
    assert!(
        sources::production_rust_files(&root).count() > 50,
        "the production-source sweep found almost nothing — the scan itself is broken, and \
         a guard that inspects nothing always passes"
    );
}

/// Pins the matcher itself against synthetic sources, not just its verdict on today's
/// tree — the anti-vacuity lesson `paper_parity_plan_section.rs` and
/// `identity_scaffolding_shared.rs`'s `matcher_distinguishes_declarations_from_mentions`
/// both name: a guard that only ever runs against a clean tree cannot tell a correct
/// matcher from one that exempts (or catches) everything.
///
/// This is also the regression pin for the bug this guard's own first run hit: a bare
/// `text.contains("derive_unwrap_secret")` flagged `crates/cairn-event/src/seal.rs` —
/// the function's OWN defining file — as an unlisted offender, because that file both
/// declares it and unit-tests it in a same-file `#[cfg(test)]` tail. Neither is the
/// #495 coupling; a real call site elsewhere in the same file must still be caught.
#[test]
fn the_matcher_exempts_the_declaration_and_test_fixtures_but_not_a_real_call() {
    // The function's own defining file: declaration + a same-file test tail calling it
    // as a fixture. Modelled on `crates/cairn-event/src/seal.rs`'s actual shape.
    let defining_file = concat!(
        "pub fn derive_unwrap_secret(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {\n",
        "    todo!()\n",
        "}\n",
        "\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    use super::*;\n",
        "    #[test]\n",
        "    fn t() {\n",
        "        let secret = derive_unwrap_secret(&seed_fixture(1));\n",
        "    }\n",
        "}\n",
    );
    assert!(
        !calls_derive_unwrap_secret(defining_file),
        "the defining file's own declaration + test-tail fixture must not read as a \
         production call"
    );

    // A real production call site, in an otherwise-identical file (a wrapper that
    // forwards to it — the #495 shape this guard exists to catch).
    let real_call_site = concat!(
        "pub fn sneaky_reintroduction(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {\n",
        "    cairn_event::seal::derive_unwrap_secret(seed)\n",
        "}\n",
    );
    assert!(
        calls_derive_unwrap_secret(real_call_site),
        "a real call site outside the cfg(test) tail and outside the declaration line \
         must still be caught"
    );

    // A call site that sits AFTER a cfg(test) tail is unrealistic in this tree (Rust
    // convention puts the test module last, and `no_drugref_dependency.rs`'s own
    // module doc states the same accepted limitation) but is worth pinning so the
    // boundary's direction — production-then-tests, not the reverse — is explicit
    // rather than assumed.
    let production_before_tail = concat!(
        "pub fn sneaky(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {\n",
        "    derive_unwrap_secret(seed)\n",
        "}\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    fn t() { let _ = derive_unwrap_secret(&[0u8; 32]); }\n",
        "}\n",
    );
    assert!(
        calls_derive_unwrap_secret(production_before_tail),
        "a real call site BEFORE the cfg(test) tail must still be caught even though the \
         tail itself also mentions the function"
    );
}
