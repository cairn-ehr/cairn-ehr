//! #527 — a cryptographic NAME is a CodeQL sink, so only real cryptography may wear one.
//!
//! ## The defect this exists to stop coming back
//!
//! `crates/cairn-medium` built its test fixtures through two helpers whose discriminator
//! parameter was called `salt`:
//!
//! ```ignore
//! pub(crate) fn salted_record(salt: u8, n: u8) -> MediumRecord { … }
//! pub(crate) fn chain_of(n: usize, salt: u8) -> (MediumV3, Vec<i64>) { … }
//! ```
//!
//! Nothing in that crate derives a key, hashes a password, or seeds an AEAD — the value is
//! a fixture discriminator, there so that two fixture chains are not byte-identical (without
//! it, "a segment spliced from ANOTHER medium" asserts nothing). But CodeQL's
//! `rust/hard-coded-cryptographic-value` decides a sink by the **name of the binding the
//! value flows into**, and every call site passed a literal:
//!
//! ```text
//! crates/cairn-medium/src/attest.rs:309  [critical]  rust/hard-coded-cryptographic-value
//!     This hard-coded value is used as a salt.
//! ```
//!
//! **Eighteen critical alerts, one per call site**, all on `main` at once.
//!
//! ## Why house rule 6's stated remedy did not help, and the correction it forces
//!
//! House rule 6 says test key material must be *computed at runtime* rather than written as
//! a literal. Both helpers already did exactly that — they build their bytes in a
//! `(0..len).map(…)` loop, the very shape the rule recommends. It made no difference,
//! because the loop is a pure function of two literal arguments: CodeQL folds straight
//! through it. The sibling helpers `testkit::bytes(seed, len)` and
//! `wire_pins::placeholder(seed, len)` run the *identical arithmetic* and are not flagged —
//! the only difference is that their parameter is called `seed`.
//!
//! So the operative rule is not only "derive it". It is also, and first:
//!
//! > **Never give a non-cryptographic value a cryptographic name.** `salt`, `nonce` and `iv`
//! > are sinks. A fixture discriminator called `salt` mints a critical alert for every call
//! > site that passes it a constant, and no amount of runtime derivation clears it.
//!
//! That is a legibility rule before it is a scanner rule: a reviewer who greps `salt` in a
//! crate that seals bodies has every reason to believe they have found a KDF.
//!
//! ## What this guard asserts
//!
//! Every binding named exactly `salt`, `nonce` or `iv` anywhere in a shipping `src/` tree is
//! named in [`ALLOWED`], where each entry states which real cryptographic construction it
//! belongs to. The list is not a suppression list — it is the inventory of the tree's actual
//! cryptography, and it is short because real cryptography in this project is confined to a
//! few files on purpose (§9: keep the safety-critical surface small).
//!
//! **WHEN THIS FAILS**, ask which of the two cases you are in:
//!
//! * The new binding really is a salt/nonce/IV of a real construction → add it to [`ALLOWED`]
//!   with the construction named, and make sure its value comes from `rand_bytes`, never a
//!   literal (that is house rule 6's other half, and this guard does not check it).
//! * It is anything else — a fixture discriminator, a counter, a tag → **rename it.** That is
//!   what `cairn-medium` did: `salted_record(salt, n)` became
//!   `distinct_record(lineage, n)`, and eighteen critical alerts went away without one byte
//!   of fixture material changing.
//!
//! ## Scope, and what is deliberately NOT checked
//!
//! * **`#[cfg(test)]` modules inside `src/` are IN scope, deliberately.** All eighteen alerts
//!   were in them. CodeQL analyses whatever the compiler is pointed at; "it is only a test"
//!   is not a distinction the alert list makes. (`tests/` directories are out of scope only
//!   because [`sources::production_rust_files`] does not walk them — see its doc for why that
//!   boundary is "does it ship", not "is it a test".)
//! * **This guard does not check that a genuine salt/nonce is randomly generated.** That is a
//!   different property with a different failure mode, and pretending one test covers both is
//!   how a guard comes to prove less than its name claims.
//! * **Block comments (`/* … */`) are not stripped**, only `//` line comments. Nothing in the
//!   tree writes a sink name inside a block comment today, which is exactly why nothing would
//!   notice if that changed; the same stated-not-implied gap `unwrap_secret_is_not_derived.rs`
//!   records for its own `#[cfg(test)]` scan.
//!
//! Uses the shared source walker (`tests/common/sources.rs`, #452) rather than a fresh one —
//! see that file's header for the two real defects a from-scratch walk has already hidden
//! behind a green guard.

#[path = "common/sources.rs"]
mod sources;

/// The binding names CodeQL's Rust cryptographic-value queries treat as sinks, and which this
/// repository therefore reserves for real cryptography.
///
/// Kept to the three that are unambiguous. `key`, `secret` and `password` are deliberately
/// ABSENT: they appear throughout the tree in compound form (`signing_key`, `unwrap_secret`,
/// `peer_key`) where they are perfectly honest, and a guard that fired on those would be
/// answered by widening [`ALLOWED`] until it meant nothing — the failure mode
/// `unwrap_secret_is_not_derived.rs` names in its own header.
const SINK_NAMES: &[&str] = &["salt", "nonce", "iv"];

/// Every file in a shipping `src/` tree that may bind a [`SINK_NAMES`] name, with the real
/// cryptographic construction that earns it.
///
/// Entries are `(repo-relative path, binding name, why it is genuinely cryptographic)`. A path
/// appears once per name, not once per occurrence: line numbers would make this list churn on
/// every unrelated edit to `seal.rs`, and the question the guard asks is about the file's
/// contents, not their position.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "crates/cairn-event/src/lib.rs",
        "nonce",
        "the pairing-offer nonce field (`make_offer`/`verify_offer`) and its unit fixtures",
    ),
    (
        "crates/cairn-event/src/seal.rs",
        "nonce",
        "the 24-byte XChaCha20-Poly1305 nonce of a sealed event body (ADR-0052)",
    ),
    (
        "crates/cairn-keystore/src/seal.rs",
        "nonce",
        "the 24-byte XChaCha20-Poly1305 nonce of the CAIRNK1 sealed key bundle",
    ),
    (
        "crates/cairn-keystore/src/seal.rs",
        "salt",
        "the 16-byte Argon2id salt that stretches CAIRN_KEY_PASSPHRASE into the KEK",
    ),
    (
        "crates/cairn-node/src/pairing.rs",
        "nonce",
        "the pairing challenge nonce — replay protection for a peering offer",
    ),
    (
        "crates/cairn-node/src/main.rs",
        "nonce",
        "the `pair-offer`/`pair-accept` CLI surface carrying that same pairing nonce",
    ),
    (
        "crates/cairn-sync/src/main.rs",
        "nonce",
        "the XChaCha20-Poly1305 nonce on the daemon's own seal/unseal path",
    ),
];

/// Which of [`SINK_NAMES`] does `line` BIND?
///
/// "Binds" means the name appears as a whole word immediately followed by a single `:` — the
/// one shape common to every way Rust introduces a name: a parameter (`fn f(salt: &[u8; 16])`),
/// a struct field (`pub nonce: [u8; 24]`), a typed local (`let nonce: [u8; 24] = …`) and a
/// struct-literal field (`nonce: "abcd".into()`). All four are sinks as far as the scanner is
/// concerned, so all four are in scope here.
///
/// Three exclusions, each load-bearing:
///
/// * **A trailing `::` is a path, not a binding** — `Nonce::from_slice`, `nonce::helper`. Those
///   flow nothing into a named slot.
/// * **Word boundaries on BOTH sides**, so `nonce_bytes:` and `recursive:` do not match. Getting
///   this wrong in the permissive direction is how a guard acquires an allow-list of unrelated
///   files and stops meaning anything.
/// * **`//` line comments are stripped first**, so this very file's prose does not fail the
///   sweep that reads it. (Block comments are not — see the module header.)
///
/// Pure and total: takes a line, returns names. That is what makes it directly testable below
/// without a filesystem, which matters because a scanner whose matcher is wrong reports a
/// confident, precise, empty answer.
fn sink_bindings(line: &str) -> Vec<&'static str> {
    let code = match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    };

    let mut found = Vec::new();
    for name in SINK_NAMES {
        if binds(code, name) {
            found.push(*name);
        }
    }
    found
}

/// Does `code` contain `name` as a whole word followed by a single `:`?
///
/// Split out from [`sink_bindings`] so the "what counts as a binding" decision is one readable
/// predicate rather than a nest of conditions inside a loop (house rule 4).
fn binds(code: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = code[from..].find(name) {
        let start = from + rel;
        let end = start + name.len();
        from = end;

        // A preceding identifier character means we matched a suffix (`nonce` inside
        // `my_nonce`), which is a different name and not a sink.
        let left_clear = code[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if !left_clear {
            continue;
        }

        // Skip spaces so `salt : u8` reads the same as `salt: u8`; Rust permits both and a
        // guard that disagreed with rustfmt on one of them would be silently narrow.
        let rest = code[end..].trim_start();
        if let Some(after_colon) = rest.strip_prefix(':') {
            // `::` is a path separator, never a binding.
            if !after_colon.starts_with(':') {
                return true;
            }
        }
    }
    false
}

#[test]
fn every_cryptographic_name_in_a_shipping_tree_is_real_cryptography() {
    let root = sources::repo_root();
    let files: Vec<_> = sources::production_rust_files(&root).collect();

    // Anti-vacuity part 1: a sweep that found almost nothing reports the same green as a
    // sweep that found everything and cleared it. The tree holds well over a hundred
    // shipping source files; 50 is a floor that a genuinely collapsed walk cannot clear.
    assert!(
        files.len() > 50,
        "the production sweep found only {} files — it has collapsed, and a collapsed \
         sweep proves nothing",
        files.len()
    );

    // (relative path, binding name) actually present in the tree.
    let mut seen: Vec<(String, &'static str)> = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        for line in sources::read_source(path).lines() {
            for name in sink_bindings(line) {
                let pair = (rel.clone(), name);
                if !seen.contains(&pair) {
                    seen.push(pair);
                }
            }
        }
    }
    seen.sort();

    // Anti-vacuity part 2: a positive control. If the matcher silently stops matching, every
    // other assertion here passes trivially. `cairn-keystore`'s Argon2id salt is the one
    // binding in this tree that is unambiguously a real cryptographic salt, so its ABSENCE
    // means the matcher broke, not that the tree got cleaner.
    assert!(
        seen.contains(&("crates/cairn-keystore/src/seal.rs".to_string(), "salt")),
        "the matcher no longer finds cairn-keystore's Argon2id salt — it is broken, and \
         every other assertion in this test is now vacuous. Found: {seen:?}"
    );

    let offenders: Vec<_> = seen
        .iter()
        .filter(|(rel, name)| {
            !ALLOWED
                .iter()
                .any(|(path, allowed, _)| path == rel && allowed == name)
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "these bindings wear a cryptographic name without being cryptography:\n  {}\n\n\
         Each one makes CodeQL's rust/hard-coded-cryptographic-value fire (critical) at \
         EVERY call site that passes it a constant — eighteen alerts at once was the #527 \
         case. If it is real cryptography, add it to ALLOWED with the construction named. \
         Otherwise RENAME it: a discriminator is not a salt.",
        offenders
            .iter()
            .map(|(rel, name)| format!("{rel}  binds `{name}`"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // Anti-vacuity part 3: a dead ALLOWED entry is worse than none — it reads as a live
    // exemption while exempting nothing, so the next reader believes the tree still holds
    // cryptography it no longer holds. Same liveness rule `unwrap_secret_is_not_derived.rs`
    // applies to its own list.
    for (path, name, why) in ALLOWED {
        assert!(
            seen.contains(&(path.to_string(), name)),
            "ALLOWED still names {path} / `{name}` ({why}), but the sweep no longer finds \
             that binding. Delete the entry — an exemption for something that is gone hides \
             the fact that it is gone."
        );
    }
}

/// The matcher's own tests. Without these, "found nothing" and "matches nothing" are the same
/// green — the exact false all-clear the slice-2a review wave spent a day removing.
#[cfg(test)]
mod matcher {
    use super::sink_bindings;

    #[test]
    fn a_parameter_a_field_a_local_and_a_literal_all_bind() {
        assert_eq!(sink_bindings("    salt: &[u8; 16],"), vec!["salt"]);
        assert_eq!(sink_bindings("    pub nonce: [u8; 24],"), vec!["nonce"]);
        assert_eq!(sink_bindings("    let nonce: [u8; 24] = x;"), vec!["nonce"]);
        assert_eq!(
            sink_bindings("        nonce: \"abcd\".into(),"),
            vec!["nonce"]
        );
        assert_eq!(sink_bindings("fn f(iv: &[u8]) {}"), vec!["iv"]);
    }

    #[test]
    fn a_longer_identifier_that_merely_contains_a_sink_name_does_not_bind() {
        // The permissive direction is the dangerous one: every one of these matching would
        // push unrelated files into ALLOWED until the list stopped carrying meaning.
        assert!(sink_bindings("    let nonce_bytes: [u8; 24] = x;").is_empty());
        assert!(sink_bindings("    let recursive: bool = true;").is_empty());
        assert!(sink_bindings("    salted: u8,").is_empty());
        assert!(sink_bindings("    let my_salt: u8 = 1;").is_empty());
    }

    #[test]
    fn a_path_separator_is_not_a_binding() {
        assert!(sink_bindings("    let n = Nonce::from_slice(&b);").is_empty());
        assert!(sink_bindings("    nonce::helper();").is_empty());
    }

    #[test]
    fn a_line_comment_is_not_code() {
        // This guard's own module header writes `salt:` in prose. If comments counted, the
        // guard would fail on the file that defines it — and the natural fix would be to
        // exempt that file, which is how a guard starts exempting the thing it guards.
        assert!(sink_bindings("// the 16-byte Argon2id salt: stretches the passphrase").is_empty());
        assert!(sink_bindings("    let x = 1; // nonce: not here").is_empty());
    }

    #[test]
    fn a_binding_before_a_comment_still_counts() {
        // The comment strip must not become a way to hide a real binding behind a trailing
        // comment on the same line.
        assert_eq!(
            sink_bindings("    salt: &[u8; 16], // the Argon2id salt"),
            vec!["salt"]
        );
    }

    #[test]
    fn whitespace_before_the_colon_is_still_a_binding() {
        assert_eq!(sink_bindings("    salt : u8,"), vec!["salt"]);
    }
}
