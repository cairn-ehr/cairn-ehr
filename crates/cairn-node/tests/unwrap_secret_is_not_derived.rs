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
//!
//! ## Known limitation: production code after a MID-FILE `#[cfg(test)]` module
//!
//! Despite its name, `cfg_test_tail_start` finds the FIRST `#[cfg(test)] mod` in the
//! file, not a trailing one — production code placed AFTER it would go unscanned. Rust
//! convention (which this repo follows throughout `crates/*/src`) puts the test module
//! last, so nothing in this tree is shaped that way today, which is exactly why nothing
//! would have noticed if it were. This is a gap in a REGRESSION NET for an accidental
//! re-coupling, not a defence against deliberate concealment —
//! `no_drugref_dependency.rs`'s own `strip_rust_cfg_test_tail` states the identical gap,
//! in the identical terms, for the identical reason.

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

/// True iff `line` is an attribute that gates the item below it on `test` — the boundary
/// between a file's production text and its same-file unit-test tail.
///
/// **Two spellings, and BOTH are needed.** The plain `#[cfg(test)]` covers every crate under
/// `crates/`. The pgrx extension gates its tests as
/// `#[cfg(any(test, feature = "pg_test"))]` instead, because pgrx runs them inside a live
/// Postgres behind a feature flag. That spelling only became relevant when the sweep widened
/// past the Cargo workspace to every shipping tree (`sources::PRODUCTION_TREES`) — and the
/// two changes have to land together. Widening the sweep alone would read
/// `extensions/cairn_pgx/src/lib.rs`'s TEST call to `derive_unwrap_secret` as production and
/// redden this guard on correct code; and the tempting fix for that red — adding the file to
/// [`ALLOWED`] — would blanket-exempt the entire in-DB extension, destroying exactly the
/// line-level scoping this guard exists to have.
///
/// Deliberately narrow: it matches `cfg(test)` and `cfg(any(test, …))`, the two shapes this
/// repository actually uses. An exotic gate (`cfg(all(...))`, a nested `any` in another
/// position) is NOT recognised, which fails in the LOUD direction — such a file's test tail
/// would be scanned as production and the guard would redden, prompting a human to widen this
/// function rather than reach for the allow-list.
fn is_a_test_gate_attribute(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("#[cfg(test)]")
        || t.starts_with("#[cfg(any(test,")
        || t.starts_with("#[cfg(any(test ,")
}

/// The line index where the FIRST test-gated `mod` block begins, if the file has one
/// — not necessarily a trailing one; see the module header's "Known limitation" for
/// what that means for production code placed after it. Same idiom as
/// `no_drugref_dependency.rs`'s `strip_rust_cfg_test_tail` (that file's doc explains the
/// convention: a `src/` file's unit tests conventionally live in a same-file tail, so
/// production code and test fixtures share one file and must be told apart by more than
/// "which file"). Scoped locally rather than shared, because this guard's need — find
/// the boundary once — is narrower than that file's, which also blanks comments and
/// string literals for an unrelated check.
fn cfg_test_tail_start(lines: &[&str]) -> Option<usize> {
    let item_line = |from: usize| -> Option<usize> {
        (from..lines.len()).find(|&i| {
            let t = lines[i].trim_start();
            !t.is_empty() && !t.starts_with('#')
        })
    };
    lines.iter().enumerate().find_map(|(i, line)| {
        if !is_a_test_gate_attribute(line) {
            return None;
        }
        let item = item_line(i + 1)?;
        let t = lines[item].trim_start();
        (t.starts_with("mod ") || t.starts_with("pub mod ") || t.starts_with("pub(crate) mod "))
            .then_some(i)
    })
}

/// Strips a same-line `//` comment tail (covers `//`, `///`, and `//!` alike — all three
/// share the two-slash prefix, so a whole doc-comment line strips to empty). Naming the
/// function in prose — a doc comment writing `` `derive_unwrap_secret` `` in backticks,
/// exactly the shape this file's own module doc uses — is not a call, the same clause
/// (a) `no_drugref_dependency.rs` carries for its own scan.
///
/// Line-oriented and quote-unaware, like the rest of this guard: a `//` INSIDE a string
/// literal (e.g. a URL) would cut the line early and could hide a real call written on
/// the same line — a false negative, which for a guard is the UNSAFE direction (a
/// missed re-coupling, not a merely-annoying extra failure). Accepted here only because
/// no production line in this tree puts `//` inside a string next to this identifier
/// today; if one ever does, this function needs widening, not a shrug. A `/* */` block
/// comment is likewise not handled at all — the identical, separately-declared gap
/// `no_drugref_dependency.rs`'s own module doc states for its scanner. Both gaps fail
/// LOUD rather than pass silently, so the real risk is a maintainer "fixing" the
/// failure by reaching for `ALLOWED` instead of widening this function — exactly the
/// wrong move this guard's declaration/test-tail scoping exists to make unnecessary.
fn strip_comment_tail(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Does this file's PRODUCTION text — excluding any trailing `#[cfg(test)]` module,
/// excluding same-line comments, and excluding `derive_unwrap_secret`'s own declaration
/// line — contain a real call to it? This is the file-level predicate the guard actually
/// means by "calls `derive_unwrap_secret`": naming it in your own signature, in a doc
/// comment, or in your own test fixtures, is not that.
fn calls_derive_unwrap_secret(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let tail_start = cfg_test_tail_start(&lines).unwrap_or(lines.len());
    lines[..tail_start].iter().any(|line| {
        let code = strip_comment_tail(line);
        code.contains("derive_unwrap_secret") && !is_the_declaration_line(code)
    })
}

/// Production files permitted to call `derive_unwrap_secret`, with the reason each is
/// allowed. A file NOT on this list calling it is the failure this guard exists for.
///
/// `crates/cairn-node/src/medication/sealed_submit.rs` is deliberately NOT here, and never
/// was an offender. When this guard was written its `ensure_unwrap_key` reached the
/// derivation only INDIRECTLY, through a `keystore::unwrap_secret(sk)` wrapper, so the file
/// never contained the literal `derive_unwrap_secret` — an allow-list entry for it would
/// have named a file that calls nothing on this list, which is misleading documentation and
/// would have made a later task's "remove this entry" step a silent no-op. ADR-0066
/// decision 6 has since gone further: `ensure_unwrap_key` no longer derives ANYTHING, it
/// only verifies that a provisioned key is registered, and `keystore::unwrap_secret` is
/// deleted. The entry is still correctly absent, now for the stronger reason.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/cairn-keystore/src/keystore.rs",
        "the ADR-0066 adoption migration (`adopt_derived_unwrap_secret`) — the one place a \
         pre-ADR-0066 node re-derives its old secret to keep its existing event_dek rows openable",
    ),
    (
        "crates/cairn-sync/src/unwrap_key.rs",
        "the pre-ADR-0066 fallback in `resolve_at_startup` — a node whose registered key IS its \
         derived one has no `.unwrap` file to load, and refusing to start would strand it. \
         Admissible ONLY because the derived key is checked against the registration first: a \
         restored node's derivation does not match and is refused. Retire this once no \
         pre-ADR-0066 node can exist — tracked by the #503 follow-up",
    ),
];

/// The pgrx test gate must be recognised, or the widened sweep turns correct test code into
/// a false offender.
///
/// `extensions/cairn_pgx/src/lib.rs` gates its tests as `#[cfg(any(test, feature =
/// "pg_test"))]` and legitimately calls `derive_unwrap_secret` inside one. Before the sweep
/// widened past the Cargo workspace that file was simply invisible; now it is scanned, and
/// only `is_a_test_gate_attribute` keeps its test tail out of the production text. This pins
/// that, so nobody "simplifies" the matcher back to a bare `#[cfg(test)]` and then reaches for
/// ALLOWED to silence the red — which would exempt the whole in-DB extension.
#[test]
fn the_pgrx_test_gate_is_recognised_as_a_test_tail() {
    let pgrx_shaped = concat!(
        "pub fn ships() {}\n",
        "#[cfg(any(test, feature = \"pg_test\"))]\n",
        "#[pg_schema]\n",
        "mod tests {\n",
        "    fn t() { let _ = derive_unwrap_secret(&seed_fixture(1)); }\n",
        "}\n",
    );
    assert!(
        !calls_derive_unwrap_secret(pgrx_shaped),
        "a call inside a pgrx-gated test module is NOT production code"
    );

    // Positive control: the same call ABOVE the gate is production and must still be caught,
    // or this test would pass against a matcher that exempts everything.
    let with_a_real_call = concat!(
        "pub fn ships() { let _ = derive_unwrap_secret(&seed_fixture(1)); }\n",
        "#[cfg(any(test, feature = \"pg_test\"))]\n",
        "mod tests {}\n",
    );
    assert!(
        calls_derive_unwrap_secret(with_a_real_call),
        "a production call must still be caught in a file that also has a pgrx test gate"
    );
}

#[test]
fn only_the_adoption_migration_derives_the_unwrap_secret() {
    let root = sources::repo_root();
    // Collected once (not re-swept per assertion below) so every check below sees
    // EXACTLY the set the offender scan itself ran against.
    let files: Vec<std::path::PathBuf> = sources::production_rust_files(&root).collect();

    // Anti-vacuity for the WIDENED sweep (review follow-up). The `> 50` floor below cannot
    // notice one missing tree, and a tree that is absent is skipped silently by design (a
    // partial checkout must not panic every guard). So assert the two trees this widening
    // was FOR are genuinely in the set — otherwise the widening could be reverted by a typo
    // in `PRODUCTION_TREES` and every guard would go on passing.
    let swept = |needle: &str| files.iter().any(|f| f.to_string_lossy().contains(needle));
    assert!(
        swept("extensions/cairn_pgx/src"),
        "the sweep must reach the in-DB extension — it ships inside Postgres and depends on \
         cairn-event, so it is exactly where a re-coupling would matter most"
    );
    assert!(
        swept("cairn-gui/"),
        "the sweep must reach the reference UI — it ships too, workspace-excluded or not"
    );

    let mut offenders: Vec<String> = Vec::new();
    // (relative path, whether that file's production text actually calls
    // `derive_unwrap_secret`) for every swept file — ONE record both the offender scan
    // and the anti-vacuity checks below read from, so "was it swept" and "does it still
    // call the function" can never quietly disagree between two separately-maintained
    // collections.
    let mut swept: Vec<(String, bool)> = Vec::new();
    // Positive control (anti-vacuity part 2 below): does the function's own
    // DECLARATION — not merely the bare identifier — still exist somewhere in the
    // sweep? Round-1 review used a bare-identifier count here, but that stays positive
    // on a leftover PROSE mention alone (e.g. a post-rename comment reading "// formerly
    // derive_unwrap_secret") even after the real function is gone — exactly the
    // silent-green failure mode this control exists to close. Pinning the literal
    // declaration text closes that: only the actual `fn derive_unwrap_secret(...)` can
    // satisfy it, a comment never can.
    let mut declaration_sightings = 0usize;

    for path in &files {
        let text = sources::read_source(path);
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");

        if text.contains("fn derive_unwrap_secret(") {
            declaration_sightings += 1;
        }

        let calls = calls_derive_unwrap_secret(&text);
        swept.push((rel.clone(), calls));

        if !calls {
            continue;
        }
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

    // Anti-vacuity, part 1: the guard must be scanning real files, not an empty set. If
    // the helper's globbing breaks, an empty sweep would pass silently and forever.
    assert!(
        files.len() > 50,
        "the production-source sweep found almost nothing — the scan itself is broken, and \
         a guard that inspects nothing always passes"
    );

    // Anti-vacuity, part 2 (positive control): the function's DECLARATION must still be
    // findable, literally, somewhere in the sweep. See `declaration_sightings`'s doc
    // comment above for exactly which silent-pass this catches, and why the bare
    // identifier (round 1's version of this check) was not strong enough.
    assert!(
        declaration_sightings > 0,
        "no swept file contains the literal `fn derive_unwrap_secret(` any more — has \
         the function been renamed, or moved out of crates/*/src? (A leftover comment \
         mentioning the old name would NOT satisfy this check — only the real \
         declaration can.) If this fires, this guard is now VACUOUS and must be updated \
         to track the new name/location, never simply deleted."
    );

    // Anti-vacuity, part 3: every live ALLOWED entry must (a) still be inside the swept
    // set, and (b) still genuinely call the function. (a) catches a walker silently
    // narrowed to fewer crates, or the file having moved. (b) catches an entry that has
    // gone INERT — the exact defect a prior review round found BY HAND in
    // `sealed_submit.rs` (removed from ALLOWED above): a file that never called
    // `derive_unwrap_secret` sitting in the list looking legitimate while protecting
    // nothing. The fix for an inert entry is always to DELETE it, never to keep it "just
    // in case" — a stale exemption is how an allow-list rots into a rubber stamp.
    for (allowed, _) in ALLOWED {
        match swept.iter().find(|(rel, _)| rel == allowed) {
            None => panic!(
                "ALLOWED names {allowed:?}, but the production-source sweep never saw \
                 that file — the walker's scope has narrowed, or the file moved, and \
                 this allow-list entry is now unverifiable"
            ),
            Some((_, calls)) => assert!(
                *calls,
                "ALLOWED names {allowed:?}, but that file no longer calls \
                 `derive_unwrap_secret` — this entry is INERT and must be DELETED, not \
                 kept \"just in case\""
            ),
        }
    }
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

    // A real call site that sits BEFORE the cfg(test) tail must still be caught, even
    // though the tail ITSELF also mentions the function as a fixture — the matcher must
    // not treat "the tail names it too" as license to ignore the production call that
    // precedes it. (The reverse case — production code placed AFTER a `#[cfg(test)]
    // mod` — is a known, separately-documented gap; see the module header.)
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

/// Regression pin for the second review-round finding: without comment stripping, ANY
/// doc comment writing `` `derive_unwrap_secret` `` in backticks — the exact shape this
/// file's own module doc uses, and the shape a future doc edit to `seal.rs`'s
/// `load_unwrap_secret`/`generate_unwrap_secret` docs could easily take — would trip the
/// guard. The natural but wrong fix for that would be adding `seal.rs` to `ALLOWED`,
/// which blanket-exempts the whole defining file and destroys the declaration/test-tail
/// line-scoping this guard exists to have. So comments must be stripped before matching,
/// and this pins that a comment-only mention stays clean while a real call right next
/// to an unrelated comment is still caught.
#[test]
fn the_matcher_exempts_comment_mentions_but_not_a_real_call_beside_one() {
    let doc_comment_only = concat!(
        "/// See `derive_unwrap_secret` for the pre-ADR-0066 adoption path.\n",
        "//! module-level mention of derive_unwrap_secret in prose\n",
        "// a line comment naming derive_unwrap_secret too\n",
        "pub fn unrelated() {}\n",
    );
    assert!(
        !calls_derive_unwrap_secret(doc_comment_only),
        "a doc/line comment naming the function in prose must not read as a call"
    );

    let real_call_beside_a_comment = concat!(
        "pub fn sneaky(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {\n",
        "    // derive_unwrap_secret is mentioned here too, but the call below is real\n",
        "    cairn_event::seal::derive_unwrap_secret(seed)\n",
        "}\n",
    );
    assert!(
        calls_derive_unwrap_secret(real_call_beside_a_comment),
        "a real call must still be caught even on a line adjacent to a comment naming \
         the same function"
    );
}
