//! #511 — the inventory of raw `Secret32` conversions, pinned rather than asserted in prose.
//!
//! WHY THIS EXISTS. `Secret32` deliberately does NOT separate one secret role from another: an
//! X25519 unwrap secret, an Ed25519 signing seed and a per-event DEK are all the same type, so
//! `Secret32::from_bytes(sk.to_bytes())` compiles. That residual was a deliberate design choice
//! (see `cairn-event/src/keys.rs`'s header), and the ENTIRE safety argument for it is that the
//! conversion stopped being an invisible coercion and became "a named, greppable line".
//!
//! That argument is only as good as the grep. Round-1 review of #511 found the count stated in
//! FOUR places with THREE different numbers, none of them correct, and the guard the prose named
//! as its pin — `unwrap_secret_is_not_derived.rs` — turned out to pin an adjacent-but-different
//! property (calls to `derive_unwrap_secret`, file-granular). Nothing counted conversions at all.
//! A reviewer who greps and finds more sites than the doc claims will either assume they misread
//! the doc or edit the number to match; both are how a live inventory becomes a stale one.
//!
//! So the inventory lives HERE, in code that fails, with the reason for each site beside it.
//!
//! WHEN THIS FAILS: do not raise the number to make it green. Ask what the new conversion is
//! turning INTO a secret. If the answer is "this node's Ed25519 signing seed", it is the ADR-0066
//! coupling — the one that emptied a restored solo clinic's whole clinical record (#495) —
//! returning by the one door #511 left open on purpose.
//!
//! WHY PER-FILE COUNTS AND NOT A FILE ALLOW-LIST. A file-granular list is what
//! `unwrap_secret_is_not_derived.rs` uses, and for its property that is right. Here it would be
//! too weak in exactly the place it matters: `cairn-keystore/src/keystore.rs` legitimately holds
//! three conversions, so a fourth added beside them — the copy-paste of `generate_sealed` that
//! writes the signing seed into the `.unwrap` file — would land inside an already-allowed file
//! and redden nothing. The count is the guard.
//!
//! Uses the shared source-inspection walker (`tests/common/sources.rs`, #452) rather than rolling
//! a new one, and pulls it in as a `#[path = ...]` leaf module exactly as
//! `unwrap_secret_is_not_derived.rs` does — see that file's header for why (a from-scratch walk
//! has twice hidden a real defect behind a green guard), and for why staying out of
//! `common/mod.rs` keeps this file's helpers out of `identity_scaffolding_shared.rs`'s derivation.
//!
//! ## Known limitation, stated in the same terms as its sibling
//!
//! `cfg_test_tail_start` finds the FIRST `#[cfg(test)] mod` in a file, not a trailing one, so
//! production code placed AFTER a mid-file test module would go unscanned. Rust convention (which
//! this repo follows throughout `crates/*/src`) puts the test module last. This is a regression
//! net against an accidental re-coupling, not a defence against deliberate concealment.

#[path = "common/sources.rs"]
mod sources;

use std::path::Path;

/// The conversion this guard counts. Written once so the matcher, the failure message and the
/// module header cannot drift apart from one another.
const NEEDLE: &str = "Secret32::from_bytes(";

/// True iff this line is a test-module gate. Deliberately narrow — it matches `cfg(test)` and
/// `cfg(any(test, …))`, the two shapes this repository actually uses, including the pgrx
/// `#[cfg(any(test, feature = "pg_test"))]` form. An exotic gate is NOT recognised, which fails
/// in the LOUD direction: such a file's test tail would be scanned as production and this guard
/// would redden, prompting a human to widen this function rather than reach for the inventory.
fn is_a_test_gate_attribute(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("#[cfg(test)]")
        || t.starts_with("#[cfg(any(test,")
        || t.starts_with("#[cfg(any(test ,")
}

/// The line index where the first test-gated `mod` block begins, if the file has one.
///
/// Same idiom as `unwrap_secret_is_not_derived.rs`'s function of the same name: a `src/` file's
/// unit tests conventionally live in a same-file tail, so production code and test fixtures share
/// one file and must be told apart by more than "which file". A gate attribute may be followed by
/// further attributes (`#[pg_schema]`) before the `mod` item itself, so the first non-attribute,
/// non-blank line after the gate is what decides.
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
        lines[item].trim_start().starts_with("mod ").then_some(i)
    })
}

/// Everything before the first `//` on the line.
///
/// A conversion NAMED in a comment — this file's own header does it repeatedly, and so does
/// `keys.rs`'s — is documentation, not a call site. Counting those would make the inventory grow
/// every time somebody explained it, which is the fastest way to teach a maintainer that the
/// number is noise.
fn strip_comment_tail(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// How many real `Secret32::from_bytes(` call sites this file's PRODUCTION text contains.
///
/// Every occurrence on a line is counted, not just the first: a line holding two conversions is
/// two conversions, and a matcher that stops at the first hit is one rustfmt run away from
/// silently under-counting — the direction that hides a site.
fn production_conversions(text: &str) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    let tail_start = cfg_test_tail_start(&lines).unwrap_or(lines.len());
    lines[..tail_start]
        .iter()
        .map(|line| strip_comment_tail(line).matches(NEEDLE).count())
        .sum()
}

/// THE INVENTORY: every production file that turns loose bytes into a `Secret32`, how many times,
/// and why each one is legitimate.
///
/// The `kind` column is the part a reviewer should read first. Only two of these turn THIS NODE'S
/// SIGNING SEED into a secret in a way that could couple custody to identity; the rest mint fresh
/// randomness or compare. Keeping them in one table, split by kind, is what the prose in
/// `keys.rs` used to attempt and get wrong.
const INVENTORY: &[(&str, usize, &str)] = &[
    (
        "crates/cairn-keystore/src/keystore.rs",
        3,
        "THREE, and only ONE of them installs anything. (1) `generate_sealed` seals the Ed25519 \
         signing seed into the signing-key file — the seed being stored AS the seed, no role \
         crossed. (2) `adopt_derived_unwrap_secret` is the ADR-0066 adoption migration, the FIRST \
         of the tree's two production lines that turn the signing seed into this node's unwrap \
         secret (the other is `cairn-sync/src/unwrap_key.rs`, below), and it is the delicate one. \
         (3) `unwrap_secret_is_the_signing_seed` COMPARES against the seed to catch a swapped \
         file; it never installs, which is why it is safe where (2) is delicate",
    ),
    (
        "crates/cairn-keystore/src/seal.rs",
        1,
        "`seal` mints a transient DEK from the OS CSPRNG to encrypt the seed under both escrow \
         secrets. Fresh randomness, no role crossed. (It routes through a bare stack array \
         rather than `Secret32::zeroed()` + fill; that is issue #544, not a role crossing)",
    ),
    (
        "crates/cairn-node/src/localstate.rs",
        1,
        "`establish_lsk` mints the local-state key from the OS CSPRNG. Fresh randomness, no role \
         crossed. Same bare-stack-array note as above — issue #544",
    ),
    (
        "crates/cairn-sync/src/unwrap_key.rs",
        1,
        "`resolve_at_startup`'s pre-ADR-0066 fallback — the SECOND and last production line that \
         turns the signing seed into an unwrap secret. Admissible only because the derived key is \
         checked against the registration first, so a restored node's derivation does not match \
         and is refused. Retire with issue #514",
    ),
];

/// The matcher, on synthetic input, before it is trusted against the tree.
///
/// The producer-count guard in `dr_clinical_guarantee_gap.rs` shipped without this and round-1
/// review found it blind to `Self { … }`; its own doc notes an earlier bug was "found by
/// mutation-testing the widened guard, not by reasoning about it". A guard whose matcher is
/// untested is a number, not a net.
#[test]
fn the_matcher_counts_call_sites_and_not_comments_or_test_fixtures() {
    let shaped = concat!(
        "// A doc line naming Secret32::from_bytes(x) is documentation, not a call.\n",
        "//! And so is Secret32::from_bytes(y) in a module header.\n",
        "pub fn real() { let _ = Secret32::from_bytes(seed); } // Secret32::from_bytes(z)\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    fn fixture() { let _ = Secret32::from_bytes(a); }\n",
        "}\n",
    );
    assert_eq!(
        production_conversions(shaped),
        1,
        "only the production call site counts — not the two comment mentions, not the \
         trailing-comment mention, and not the test fixture"
    );

    // Two on one line must count as two, or a rustfmt join silently shrinks the inventory.
    assert_eq!(
        production_conversions("let p = (Secret32::from_bytes(a), Secret32::from_bytes(b));\n"),
        2,
        "every occurrence on a line counts, not just the first"
    );

    // The pgrx gate must be recognised, or the sweep turns correct in-DB test code into an
    // offender and the next author reaches for the inventory to silence it.
    let pgrx_shaped = concat!(
        "pub fn ships() {}\n",
        "#[cfg(any(test, feature = \"pg_test\"))]\n",
        "#[pg_schema]\n",
        "mod tests {\n",
        "    fn t() { let _ = Secret32::from_bytes(a); }\n",
        "}\n",
    );
    assert_eq!(
        production_conversions(pgrx_shaped),
        0,
        "a conversion inside a pgrx-gated test module is not production code"
    );

    // Positive control: the same call ABOVE the gate is production and must still be counted,
    // or this test would pass against a matcher that exempts everything.
    let above_the_gate = concat!(
        "pub fn ships() { let _ = Secret32::from_bytes(a); }\n",
        "#[cfg(any(test, feature = \"pg_test\"))]\n",
        "mod tests {}\n",
    );
    assert_eq!(
        production_conversions(above_the_gate),
        1,
        "a production conversion must still be counted in a file that also has a test gate"
    );
}

#[test]
fn every_production_secret32_conversion_is_in_the_inventory() {
    let root = sources::repo_root();
    // Collected once (not re-swept per assertion) so every check below sees EXACTLY the set the
    // inventory scan itself ran against.
    let files: Vec<std::path::PathBuf> = sources::production_rust_files(&root).collect();

    // Anti-vacuity for the sweep's REACH. The floor below cannot notice one missing tree, and a
    // tree that is absent is skipped silently by design (a partial checkout must not panic every
    // guard). So assert the two workspace-EXCLUDED trees are genuinely in the set — they are the
    // ones no root `cargo` gate sees, and `extensions/cairn_pgx` ships inside Postgres.
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

    let mut found: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let n = production_conversions(&sources::read_source(path));
        if n == 0 {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        found.push((rel, n));
    }
    found.sort();

    let mut expected: Vec<(String, usize)> = INVENTORY
        .iter()
        .map(|(path, n, _)| ((*path).to_string(), *n))
        .collect();
    expected.sort();

    assert_eq!(
        found, expected,
        "the production `Secret32::from_bytes` inventory moved. Do NOT edit the numbers to match \
         — ask what the new conversion turns into a secret. If it is this node's Ed25519 signing \
         seed, it is the ADR-0066 coupling returning (#495): the restored solo clinic that could \
         open none of its own record. If it is fresh CSPRNG output, add it to INVENTORY with that \
         reason. Found: {found:?}, inventory: {expected:?}"
    );

    // Anti-vacuity: the guard must be scanning real files, not an empty set. If the helper's
    // globbing breaks, an empty sweep would agree with an empty inventory and pass for ever.
    assert!(
        files.len() > 50,
        "the source sweep collected only {} files — the walker is broken, and a guard that \
         scans nothing agrees with any inventory",
        files.len()
    );

    // Anti-vacuity, part 2: the NEEDLE must still name something real. If `Secret32::from_bytes`
    // were renamed, every count would drop to zero, the inventory would be edited to match, and
    // this guard would go on passing while guarding nothing.
    assert!(
        !found.is_empty(),
        "no production file converts raw bytes into a `Secret32` at all — either the constructor \
         was renamed (update NEEDLE) or the sweep is broken. Both leave this guard vacuous"
    );
}
