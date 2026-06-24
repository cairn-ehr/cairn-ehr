# Node Keystore Seal + Recovery Escrow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Seal `cairn-node`'s Ed25519 signing key at rest with a dual-recipient envelope (operational passphrase + one-time off-node recovery code) and mint the recovery escrow at provisioning, closing the `key_at_rest PLAINTEXT` / `dr_escrow STUBBED` honest gaps.

**Architecture:** A new pure `seal.rs` module holds the safety-critical crypto (random DEK seals the seed via XChaCha20-Poly1305; the DEK is wrapped twice under Argon2id-derived KEKs — one per secret — and the whole thing serializes to versioned CBOR with a magic header). `keystore.rs` becomes a thin adapter that writes/reads that format and auto-detects sealed-vs-plaintext. `main.rs` wires the passphrase/recovery-code UX. No DB, schema, or event-format change.

**Tech Stack:** Rust (cairn-node crate), `argon2` (Argon2id KDF), `chacha20poly1305` (XChaCha20-Poly1305 AEAD), `ciborium` (CBOR), `rpassword` (passphrase prompt), `getrandom::fill` (entropy), `tokio-postgres` (existing, integration test only).

## Global Constraints

- **MSRV:** Rust 1.74 (`workspace.package.rust-version`). Verify new deps build on it.
- **License:** AGPL-3.0-only. Every new dep must be AGPL-3.0-compatible — `argon2`, `chacha20poly1305`, `ciborium` are dual MIT/Apache-2.0; `rpassword` is Apache-2.0. All compatible. Confirm before adding (`cargo tree`/crates.io).
- **TDD:** failing test first, then minimal code, then green, then commit. Load-bearing here — this is the §9 safety-critical surface.
- **Entropy:** draw randomness via `getrandom::fill(&mut buf)` (the getrandom 0.4 API already used in `cairn-event::generate_key`). Do NOT pull `OsRng`/a second getrandom version (issue #11 dedup).
- **Inline docs:** every non-trivial fn carries a junior-readable comment explaining *why* it exists and how it fits.
- **Files under ~500 LOC.** `seal.rs` is the only sizeable new file; keep it focused.
- **No silent plaintext:** `init`/`seal-key` refuse to write an unsealed key unless `--insecure-plaintext` is explicitly passed.

---

## File Structure

- **Create** `crates/cairn-node/src/seal.rs` — pure seal/unseal, CBOR (de)serialization, base32 + recovery-code generation. Safety-critical, no I/O except entropy.
- **Modify** `crates/cairn-node/src/lib.rs` — add `pub mod seal;`.
- **Modify** `crates/cairn-node/src/keystore.rs` — `generate_plaintext`, `generate_sealed`, `seal_existing`, `load` (auto-detect), `key_at_rest_state` + `KeyAtRest` enum. Remove `generate_and_seal`.
- **Modify** `crates/cairn-node/src/identity.rs` — `Status.recovery_escrow`; `status()` uses `key_at_rest_state` instead of `load(..,None)`; honest `key_at_rest`/`dr_escrow` strings.
- **Modify** `crates/cairn-node/src/main.rs` — `Init` flags (`--passphrase`, `--insecure-plaintext`), new `SealKey` subcommand, `resolve_passphrase`/`load_signing_key` helpers, daemon commands load with the secret, status prints `recovery_escrow`.
- **Modify** `crates/cairn-node/Cargo.toml` — add `argon2`, `chacha20poly1305`, `ciborium`, `rpassword`.
- **Modify** existing tests calling `generate_and_seal(_, None)` → `generate_plaintext(_)`: `tests/status.rs`, `tests/sync_watermark.rs`, `tests/genesis_hlc.rs`, `tests/floor_enforced.rs`, `tests/federation.rs`, `tests/pairing.rs`, `tests/admission.rs`, `tests/provision.rs`.
- **Create** `crates/cairn-node/tests/keystore_seal.rs` — DB-gated integration test for the sealed `init` + escrow + status path.

---

## Task 1: Crypto deps + base32 + recovery-code generator

**Files:**
- Modify: `crates/cairn-node/Cargo.toml`
- Create: `crates/cairn-node/src/seal.rs`
- Modify: `crates/cairn-node/src/lib.rs`

**Interfaces:**
- Produces: `seal::base32_encode(&[u8]) -> String`, `seal::base32_decode(&str) -> Option<Vec<u8>>`, `seal::normalize_recovery_code(&str) -> String`, `seal::generate_recovery_code() -> String`.

- [ ] **Step 1: Add dependencies**

Edit `crates/cairn-node/Cargo.toml`, in `[dependencies]` after `base64`:

```toml
# At-rest key sealing (ADR-0026 slice A). All dual MIT/Apache-2.0 except
# rpassword (Apache-2.0) — AGPL-3.0-compatible. Pure-Rust, reviewer-legible (§9).
argon2 = "0.5"             # Argon2id KDF: passphrase/recovery-code -> key-encryption-key
chacha20poly1305 = "0.10"  # XChaCha20-Poly1305 AEAD (192-bit nonce: random nonce, no reuse worry)
ciborium = "0.2"           # CBOR (de)serialization of the sealed-key bundle (same as cairn-event)
getrandom = "0.4"          # salts/nonces/DEK entropy (matches cairn-event; no OsRng, issue #11)
rpassword = "7"            # no-echo operational-passphrase prompt
```

- [ ] **Step 2: Register the module**

Edit `crates/cairn-node/src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod seal;
```

- [ ] **Step 3: Write the failing tests (base32 + recovery code)**

Create `crates/cairn-node/src/seal.rs` with ONLY this test module for now:

```rust
//! At-rest key sealing for cairn-node (ADR-0026 slice A).
//!
//! WHY THIS EXISTS: a node's Ed25519 signing key must survive on disk without being
//! readable by anyone who copies the file, and must be recoverable off-node after a
//! lost passphrase or a dead disk. This module is the small safety-critical surface
//! ADR-0026 names: pure functions (entropy aside) that seal a 32-byte seed under TWO
//! independent secrets — an operational passphrase (daily, unattended `run`) and a
//! one-time recovery code (paper escrow). A defect here is silent key loss or a
//! forged identity, so it is exhaustively unit-tested and kept reviewer-legible.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_roundtrips_arbitrary_bytes() {
        for v in [vec![], vec![0u8], vec![0xff; 20], (0u8..=255).collect::<Vec<_>>()] {
            let enc = base32_encode(&v);
            assert_eq!(base32_decode(&enc).as_deref(), Some(v.as_slice()),
                       "roundtrip failed for {} bytes", v.len());
        }
    }

    #[test]
    fn base32_rejects_invalid_chars() {
        // 'I','L','O','U' are excluded from Crockford base32; a literal '!' is invalid.
        assert!(base32_decode("!!!!").is_none());
    }

    #[test]
    fn normalize_strips_grouping_and_case() {
        assert_eq!(normalize_recovery_code("ab cde-fghjk"), "ABCDEFGHJK");
    }

    #[test]
    fn recovery_code_is_160_bit_and_unique() {
        let a = generate_recovery_code();
        let b = generate_recovery_code();
        assert_ne!(a, b, "two codes must differ (entropy smoke test)");
        // Decodes to exactly 20 bytes (160 bits).
        assert_eq!(base32_decode(&normalize_recovery_code(&a)).map(|v| v.len()), Some(20));
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p cairn-node --lib seal:: 2>&1 | tail -20`
Expected: FAIL — `base32_encode`/`base32_decode`/`normalize_recovery_code`/`generate_recovery_code` not found.

- [ ] **Step 5: Implement base32 + recovery code (pure)**

In `crates/cairn-node/src/seal.rs`, above the test module, add:

```rust
/// Crockford base32 alphabet (excludes I, L, O, U to avoid transcription errors).
const B32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode bytes as Crockford base32 (no padding). Pure. Used to render the
/// 160-bit recovery code as a human-transcribable string.
pub fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((buf >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// Decode Crockford base32; `None` on any character outside the alphabet.
/// Input must already be normalized (uppercase, no separators).
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let (mut buf, mut bits) = (0u32, 0u32);
    let mut out = Vec::new();
    for c in s.chars() {
        let idx = B32.iter().position(|&a| a as char == c)? as u32;
        buf = (buf << 5) | idx;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Canonical form of a recovery code for KDF input: uppercase, keep only
/// alphabet characters (drops grouping dashes/spaces and lowercases). This lets a
/// human re-type the code with any spacing/case and still unseal.
pub fn normalize_recovery_code(s: &str) -> String {
    s.to_ascii_uppercase().chars().filter(|c| B32.contains(&(*c as u8))).collect()
}

/// Generate a fresh 160-bit recovery code, grouped in 5-char blocks for legibility,
/// e.g. `AB12C-D34EF-...`. Shown ONCE at provisioning; the off-node escrow.
pub fn generate_recovery_code() -> String {
    let mut raw = [0u8; 20];
    getrandom::fill(&mut raw).expect("entropy source unavailable");
    let flat = base32_encode(&raw);
    flat.as_bytes()
        .chunks(5)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --lib seal:: 2>&1 | tail -20`
Expected: PASS (4 tests).

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/Cargo.toml crates/cairn-node/src/seal.rs crates/cairn-node/src/lib.rs crates/cairn-node/Cargo.lock
git commit -m "feat(seal): base32 + 160-bit recovery code generator (ADR-0026 slice A)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Seal / unseal core + CBOR serialization

**Files:**
- Modify: `crates/cairn-node/src/seal.rs`

**Interfaces:**
- Consumes: `normalize_recovery_code` (Task 1).
- Produces:
  - `seal::SealedKey` (serde Serialize/Deserialize)
  - `seal::SealError` (thiserror)
  - `seal::seal(seed: &[u8;32], op_pass: &str, recovery_code: &str) -> Result<SealedKey, SealError>`
  - `seal::unseal(s: &SealedKey, secret: &str) -> Option<[u8;32]>`
  - `seal::to_cbor(s: &SealedKey) -> Vec<u8>` (magic-prefixed)
  - `seal::from_cbor(bytes: &[u8]) -> Result<SealedKey, SealError>`
  - `SealedKey::has_recovery_wrap(&self) -> bool`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/cairn-node/src/seal.rs`:

```rust
    const SEED: [u8; 32] = [7u8; 32];

    #[test]
    fn unseals_with_operational_passphrase() {
        let s = seal(&SEED, "op-pass", "REC-CODE").unwrap();
        assert_eq!(unseal(&s, "op-pass"), Some(SEED));
    }

    #[test]
    fn unseals_with_recovery_code_any_formatting() {
        let s = seal(&SEED, "op-pass", "ab12c-d34ef").unwrap();
        // re-typed with different case/spacing still works
        assert_eq!(unseal(&s, "AB12C D34EF"), Some(SEED));
    }

    #[test]
    fn wrong_secret_returns_none() {
        let s = seal(&SEED, "op-pass", "REC-CODE").unwrap();
        assert_eq!(unseal(&s, "nope"), None);
    }

    #[test]
    fn tampered_fields_return_none() {
        let base = seal(&SEED, "op-pass", "REC-CODE").unwrap();
        let mutate = |f: fn(&mut SealedKey)| { let mut s = base.clone(); f(&mut s); s };
        // Flipping a byte in any ciphertext, salt, or nonce must fail the AEAD tag.
        assert_eq!(unseal(&mutate(|s| s.seed_ct[0] ^= 1), "op-pass"), None);
        assert_eq!(unseal(&mutate(|s| s.wrap_op.ct[0] ^= 1), "op-pass"), None);
        assert_eq!(unseal(&mutate(|s| s.wrap_rec.ct[0] ^= 1), "REC-CODE"), None);
        assert_eq!(unseal(&mutate(|s| s.salt_op[0] ^= 1), "op-pass"), None);
        assert_eq!(unseal(&mutate(|s| s.seed_nonce[0] ^= 1), "op-pass"), None);
    }

    #[test]
    fn cbor_roundtrips_and_has_magic() {
        let s = seal(&SEED, "op-pass", "REC-CODE").unwrap();
        let bytes = to_cbor(&s);
        assert!(bytes.starts_with(b"CAIRNK1\n"), "magic header must be present");
        let back = from_cbor(&bytes).unwrap();
        assert_eq!(unseal(&back, "op-pass"), Some(SEED));
        assert!(back.has_recovery_wrap());
    }

    #[test]
    fn from_cbor_rejects_garbage_and_plaintext() {
        assert!(from_cbor(b"not a sealed bundle").is_err());
        assert!(from_cbor(&[0u8; 32]).is_err(), "a raw 32-byte seed is not a bundle");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-node --lib seal:: 2>&1 | tail -20`
Expected: FAIL — `SealedKey`, `seal`, `unseal`, `to_cbor`, `from_cbor` not found.

- [ ] **Step 3: Implement the seal core**

In `crates/cairn-node/src/seal.rs`, above the test module, add:

```rust
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

/// Magic header so a sealed bundle is distinguishable from a raw 32-byte plaintext
/// seed by inspection (keystore auto-detect). Versioned: bump on a format change.
const MAGIC: &[u8] = b"CAIRNK1\n";

#[derive(thiserror::Error, Debug)]
pub enum SealError {
    #[error("kdf: {0}")] Kdf(String),
    #[error("aead")] Aead,
    #[error("entropy: {0}")] Entropy(String),
    #[error("decode: {0}")] Decode(String),
}

/// One AEAD-wrapped copy of the Data Encryption Key under a single secret's KEK.
#[derive(Clone, Serialize, Deserialize)]
pub struct Wrap {
    pub nonce: [u8; 24],
    pub ct: Vec<u8>,
}

/// Argon2id cost parameters, stored in-file so a future cost change stays
/// backward-readable (forward compat).
#[derive(Clone, Serialize, Deserialize)]
pub struct ArgonParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for ArgonParams {
    fn default() -> Self {
        // RustCrypto Argon2 defaults: 19 MiB, 2 passes, 1 lane. Adequate for an
        // at-rest key wrap; tunable later without breaking old files (params are in-file).
        Self { m_cost: 19_456, t_cost: 2, p_cost: 1 }
    }
}

/// A dual-recipient sealed signing key. A random DEK encrypts the seed; the DEK is
/// wrapped once per secret (operational passphrase, recovery code). Either secret
/// recovers the DEK and hence the seed; neither secret is stored.
#[derive(Clone, Serialize, Deserialize)]
pub struct SealedKey {
    pub version: u8,
    pub argon: ArgonParams,
    pub salt_op: [u8; 16],
    pub salt_rec: [u8; 16],
    pub seed_nonce: [u8; 24],
    pub seed_ct: Vec<u8>,
    pub wrap_op: Wrap,
    pub wrap_rec: Wrap,
}

impl SealedKey {
    /// Whether this bundle carries a recovery-code wrap (the off-node escrow).
    /// Always true for `seal`-produced bundles; kept explicit for `status`.
    pub fn has_recovery_wrap(&self) -> bool {
        !self.wrap_rec.ct.is_empty()
    }
}

fn rand_bytes<const N: usize>() -> Result<[u8; N], SealError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| SealError::Entropy(e.to_string()))?;
    Ok(b)
}

/// Derive a 32-byte key-encryption-key from a secret + salt via Argon2id.
fn derive_kek(secret: &str, salt: &[u8; 16], p: &ArgonParams) -> Result<[u8; 32], SealError> {
    let params = Params::new(p.m_cost, p.t_cost, p.p_cost, Some(32))
        .map_err(|e| SealError::Kdf(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = [0u8; 32];
    argon.hash_password_into(secret.as_bytes(), salt, &mut kek)
        .map_err(|e| SealError::Kdf(e.to_string()))?;
    Ok(kek)
}

fn aead_encrypt(key: &[u8; 32], nonce: &[u8; 24], pt: &[u8]) -> Result<Vec<u8>, SealError> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher.encrypt(XNonce::from_slice(nonce), pt).map_err(|_| SealError::Aead)
}

fn aead_decrypt(key: &[u8; 32], nonce: &[u8; 24], ct: &[u8]) -> Option<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher.decrypt(XNonce::from_slice(nonce), ct).ok()
}

/// Wrap one DEK copy under a secret. The recovery code is normalized first so any
/// spacing/case the human re-types still derives the same KEK.
fn wrap_dek(dek: &[u8; 32], secret: &str, salt: &[u8; 16], p: &ArgonParams)
    -> Result<Wrap, SealError> {
    let kek = derive_kek(secret, salt, p)?;
    let nonce = rand_bytes::<24>()?;
    let ct = aead_encrypt(&kek, &nonce, dek)?;
    Ok(Wrap { nonce, ct })
}

fn try_unwrap(w: &Wrap, secret: &str, salt: &[u8; 16], p: &ArgonParams) -> Option<[u8; 32]> {
    let kek = derive_kek(secret, salt, p).ok()?;
    let dek = aead_decrypt(&kek, &w.nonce, &w.ct)?;
    dek.try_into().ok()
}

/// Seal a 32-byte signing seed under two independent secrets (dual-recipient).
pub fn seal(seed: &[u8; 32], op_pass: &str, recovery_code: &str) -> Result<SealedKey, SealError> {
    let argon = ArgonParams::default();
    let dek = rand_bytes::<32>()?;
    let seed_nonce = rand_bytes::<24>()?;
    let seed_ct = aead_encrypt(&dek, &seed_nonce, seed)?;
    let salt_op = rand_bytes::<16>()?;
    let salt_rec = rand_bytes::<16>()?;
    let wrap_op = wrap_dek(&dek, op_pass, &salt_op, &argon)?;
    // Normalize the recovery code so re-typing with different spacing/case unseals.
    let wrap_rec = wrap_dek(&dek, &normalize_recovery_code(recovery_code), &salt_rec, &argon)?;
    Ok(SealedKey {
        version: 1, argon, salt_op, salt_rec, seed_nonce, seed_ct, wrap_op, wrap_rec,
    })
}

/// Recover the seed from either recipient. Tries the operational passphrase first
/// (byte-exact), then the recovery code (normalized). `None` on wrong secret or any
/// tamper (the AEAD tag fails). The two paths are indistinguishable to the caller:
/// we never leak which recipient matched or why a decrypt failed.
pub fn unseal(s: &SealedKey, secret: &str) -> Option<[u8; 32]> {
    let dek = try_unwrap(&s.wrap_op, secret, &s.salt_op, &s.argon)
        .or_else(|| try_unwrap(&s.wrap_rec, &normalize_recovery_code(secret), &s.salt_rec, &s.argon))?;
    let seed = aead_decrypt(&dek, &s.seed_nonce, &s.seed_ct)?;
    seed.try_into().ok()
}

/// Serialize to magic-prefixed CBOR bytes for on-disk storage.
pub fn to_cbor(s: &SealedKey) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    ciborium::into_writer(s, &mut out).expect("CBOR serialization of SealedKey cannot fail");
    out
}

/// Parse magic-prefixed CBOR bytes. Errors (not panics) on a missing magic header
/// or malformed body — so `keystore::load` can fall through to the plaintext path.
pub fn from_cbor(bytes: &[u8]) -> Result<SealedKey, SealError> {
    let body = bytes.strip_prefix(MAGIC)
        .ok_or_else(|| SealError::Decode("missing CAIRNK1 magic".into()))?;
    ciborium::from_reader(body).map_err(|e| SealError::Decode(e.to_string()))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --lib seal:: 2>&1 | tail -30`
Expected: PASS (all seal tests). Argon2id makes these ~tens of ms each — that's expected.

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy -p cairn-node --all-targets 2>&1 | tail -20`
Expected: no warnings in `seal.rs`.

```bash
git add crates/cairn-node/src/seal.rs
git commit -m "feat(seal): dual-recipient seal/unseal + CBOR bundle (ADR-0026 slice A)

Random DEK seals the seed (XChaCha20-Poly1305); DEK wrapped twice under
Argon2id KEKs (operational passphrase + recovery code). Tamper-evident
by AEAD tag; magic-prefixed CBOR so keystore can auto-detect sealed vs plaintext.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: keystore.rs — sealed/plaintext generation, auto-detect load, migration, state inspection

**Files:**
- Modify: `crates/cairn-node/src/keystore.rs`
- Modify (mechanical rename `generate_and_seal(_, None)` → `generate_plaintext(_)`): `tests/status.rs`, `tests/sync_watermark.rs`, `tests/genesis_hlc.rs`, `tests/floor_enforced.rs`, `tests/federation.rs`, `tests/pairing.rs`, `tests/admission.rs`, `tests/provision.rs`

**Interfaces:**
- Consumes: `seal::{seal, unseal, to_cbor, from_cbor, SealedKey}` (Task 2).
- Produces:
  - `keystore::generate_plaintext(path: &Path) -> Result<(SigningKey, String), KeystoreError>`
  - `keystore::generate_sealed(path: &Path, op_pass: &str, recovery_code: &str) -> Result<(SigningKey, String), KeystoreError>`
  - `keystore::seal_existing(path: &Path, op_pass: &str, recovery_code: &str) -> Result<(), KeystoreError>`
  - `keystore::load(path: &Path, secret: Option<&str>) -> Result<SigningKey, KeystoreError>` (auto-detect)
  - `keystore::KeyAtRest` enum: `Sealed { dual_recipient: bool }`, `Plaintext`, `Missing`, `Corrupt`
  - `keystore::key_at_rest_state(path: &Path) -> KeyAtRest`

- [ ] **Step 1: Write the failing unit tests**

Add a test module at the bottom of `crates/cairn-node/src/keystore.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sealed_key_roundtrips_via_both_secrets() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let (sk, _kid) = generate_sealed(&p, "op", "REC-CODE").unwrap();
        assert_eq!(load(&p, Some("op")).unwrap().to_bytes(), sk.to_bytes());
        assert_eq!(load(&p, Some("REC-CODE")).unwrap().to_bytes(), sk.to_bytes());
        assert!(load(&p, None).is_err(), "sealed key with no secret must error");
        assert!(load(&p, Some("wrong")).is_err());
        assert!(matches!(key_at_rest_state(&p), KeyAtRest::Sealed { dual_recipient: true }));
    }

    #[test]
    fn plaintext_key_loads_without_secret() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let (sk, _kid) = generate_plaintext(&p).unwrap();
        assert_eq!(load(&p, None).unwrap().to_bytes(), sk.to_bytes());
        assert!(matches!(key_at_rest_state(&p), KeyAtRest::Plaintext));
    }

    #[test]
    fn seal_existing_migrates_plaintext_then_blocks_plaintext_load() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let (sk, _kid) = generate_plaintext(&p).unwrap();
        seal_existing(&p, "op", "REC-CODE").unwrap();
        assert_eq!(load(&p, Some("op")).unwrap().to_bytes(), sk.to_bytes());
        assert!(load(&p, None).is_err(), "after sealing, no-secret load must fail");
        assert!(seal_existing(&p, "op", "REC-CODE").is_err(), "double-seal must error");
    }

    #[test]
    fn state_reports_missing_and_corrupt() {
        let dir = tempdir().unwrap();
        assert!(matches!(key_at_rest_state(&dir.path().join("nope.key")), KeyAtRest::Missing));
        let bad = dir.path().join("bad.key");
        std::fs::write(&bad, b"only 5").unwrap(); // not 32 bytes, not a bundle
        assert!(matches!(key_at_rest_state(&bad), KeyAtRest::Corrupt));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-node --lib keystore:: 2>&1 | tail -20`
Expected: FAIL — `generate_sealed`/`generate_plaintext`/`seal_existing`/`key_at_rest_state`/`KeyAtRest` not found.

- [ ] **Step 3: Rewrite keystore.rs**

Replace the entire contents of `crates/cairn-node/src/keystore.rs` (above the test module from Step 1) with:

```rust
use crate::seal;
use cairn_event::{generate_key, SigningKey};
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum KeystoreError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("key material: {0}")] Key(String),
}

/// At-rest posture of a key file, inspectable WITHOUT the secret (for `status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAtRest {
    /// A valid sealed bundle. `dual_recipient` is true when it carries a recovery wrap.
    Sealed { dual_recipient: bool },
    /// A raw 32-byte Ed25519 seed (legacy/insecure).
    Plaintext,
    /// No file at the path.
    Missing,
    /// A file exists but is neither a sealed bundle nor a 32-byte seed.
    Corrupt,
}

/// Generate a keypair and write the seed UNSEALED (mode 0600). Insecure — only for
/// throwaway test nodes and the explicit `--insecure-plaintext` path. The recovery
/// escrow does NOT exist for a plaintext key (key loss = node loss).
pub fn generate_plaintext(path: &Path) -> Result<(SigningKey, String), KeystoreError> {
    let (sk, kid) = generate_key().map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_key_file(path, &sk.to_bytes())?;
    Ok((sk, kid))
}

/// Generate a keypair and write it SEALED under both secrets (ADR-0026 slice A).
/// The caller supplies (and is responsible for displaying) the recovery code.
pub fn generate_sealed(path: &Path, op_pass: &str, recovery_code: &str)
    -> Result<(SigningKey, String), KeystoreError> {
    let (sk, kid) = generate_key().map_err(|e| KeystoreError::Key(e.to_string()))?;
    let sealed = seal::seal(&sk.to_bytes(), op_pass, recovery_code)
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_key_file(path, &seal::to_cbor(&sealed))?;
    Ok((sk, kid))
}

/// Migrate an existing plaintext key file to the sealed format. Errors if the file
/// is already sealed (no double-seal) or is not a 32-byte seed.
pub fn seal_existing(path: &Path, op_pass: &str, recovery_code: &str) -> Result<(), KeystoreError> {
    let bytes = std::fs::read(path)?;
    if seal::from_cbor(&bytes).is_ok() {
        return Err(KeystoreError::Key("key is already sealed".into()));
    }
    let seed: [u8; 32] = bytes.as_slice().try_into()
        .map_err(|_| KeystoreError::Key("not a 32-byte plaintext key".into()))?;
    let sealed = seal::seal(&seed, op_pass, recovery_code)
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_key_file(path, &seal::to_cbor(&sealed))?;
    Ok(())
}

/// Load the signing key, auto-detecting sealed vs plaintext. A sealed file requires
/// `secret` (operational passphrase OR recovery code); a missing/wrong secret yields
/// a legible error, never a panic. A plaintext file ignores `secret`.
pub fn load(path: &Path, secret: Option<&str>) -> Result<SigningKey, KeystoreError> {
    let bytes = std::fs::read(path)?;
    if let Ok(sealed) = seal::from_cbor(&bytes) {
        let secret = secret.ok_or_else(|| KeystoreError::Key(
            "key is sealed: provide the passphrase (set CAIRN_KEY_PASSPHRASE)".into()))?;
        let seed = seal::unseal(&sealed, secret).ok_or_else(|| KeystoreError::Key(
            "cannot unseal key: wrong passphrase/recovery code or corrupt file".into()))?;
        Ok(SigningKey::from_bytes(&seed))
    } else {
        let seed: [u8; 32] = bytes.as_slice().try_into()
            .map_err(|_| KeystoreError::Key("not a sealed bundle and not a 32-byte seed".into()))?;
        Ok(SigningKey::from_bytes(&seed))
    }
}

/// Inspect the at-rest posture without needing the secret (for `status`).
pub fn key_at_rest_state(path: &Path) -> KeyAtRest {
    match std::fs::read(path) {
        Err(_) => KeyAtRest::Missing,
        Ok(bytes) => {
            if let Ok(sealed) = seal::from_cbor(&bytes) {
                KeyAtRest::Sealed { dual_recipient: sealed.has_recovery_wrap() }
            } else if bytes.len() == 32 {
                KeyAtRest::Plaintext
            } else {
                KeyAtRest::Corrupt
            }
        }
    }
}

#[cfg(unix)]
fn write_key_file(path: &Path, bytes: &[u8]) -> Result<(), KeystoreError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(bytes)?;
    Ok(())
}
#[cfg(not(unix))]
fn write_key_file(path: &Path, bytes: &[u8]) -> Result<(), KeystoreError> {
    std::fs::write(path, bytes)?;
    Ok(())
}
```

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test -p cairn-node --lib keystore:: 2>&1 | tail -20`
Expected: PASS (4 tests).

- [ ] **Step 5: Migrate existing test call sites**

In each listed test file, replace every `keystore::generate_and_seal(<path>, None)` with `keystore::generate_plaintext(<path>)`. These tests don't exercise sealing and benefit from plaintext's speed (Argon2id is deliberately slow). Run this to find any stragglers:

Run: `grep -rn "generate_and_seal" crates/`
Expected: NO matches (all renamed; the old fn is gone).

- [ ] **Step 6: Build the whole crate (tests included) to catch signature breaks**

Run: `cargo build -p cairn-node --all-targets 2>&1 | tail -20`
Expected: builds clean. (`identity.rs`/`main.rs` still call the removed `load(_, None)` fine — `load` kept its signature — but `status` in `identity.rs` is updated in Task 4; it still compiles now because `load` exists.)

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/keystore.rs crates/cairn-node/tests/
git commit -m "feat(keystore): sealed/plaintext generation, auto-detect load, migration, state (ADR-0026 slice A)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: status surfacing (identity.rs)

**Files:**
- Modify: `crates/cairn-node/src/identity.rs` (the `Status` struct ~149-176 and `status()` ~183-237)

**Interfaces:**
- Consumes: `keystore::{key_at_rest_state, KeyAtRest}` (Task 3).
- Produces: `Status.recovery_escrow: bool`; honest `key_at_rest`/`dr_escrow` strings.

- [ ] **Step 1: Add the failing assertion to the existing status test**

In `crates/cairn-node/tests/status.rs`, the first test (`status_reports_peers_and_keystore_health`) provisions with `generate_plaintext` (after Task 3), so it should still report PLAINTEXT/STUBBED. Add one assertion after the existing `key_at_rest` check (~line 71) to pin the new field for the plaintext case:

```rust
    assert!(!st.recovery_escrow, "plaintext key has no recovery escrow");
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --test status 2>&1 | tail -20`
Expected: FAIL — `Status` has no field `recovery_escrow`.

- [ ] **Step 3: Implement**

In `crates/cairn-node/src/identity.rs`, add the field to `Status` (after `dr_escrow`):

```rust
    /// `true` iff the at-rest key carries an off-node recovery wrap (ADR-0026 escrow).
    pub recovery_escrow: bool,
```

Then in `status()`, replace the keystore-health block:

```rust
    // Keystore health: try to load the key; a missing/invalid file is not an error.
    let keystore_ok = crate::keystore::load(key_path, None).is_ok();
```

with:

```rust
    // At-rest posture, inspected WITHOUT the secret (a sealed key cannot be loaded
    // here — we have no passphrase in `status` — so we classify the file instead).
    let kstate = crate::keystore::key_at_rest_state(key_path);
    use crate::keystore::KeyAtRest;
    let keystore_ok = matches!(kstate, KeyAtRest::Sealed { .. } | KeyAtRest::Plaintext);
    let (key_at_rest, recovery_escrow) = match kstate {
        KeyAtRest::Sealed { dual_recipient } => (
            format!("SEALED (argon2id + xchacha20poly1305{})",
                    if dual_recipient { "; dual-recipient" } else { "" }),
            dual_recipient,
        ),
        KeyAtRest::Plaintext => ("PLAINTEXT (0600; run `cairn-node seal-key`)".to_string(), false),
        KeyAtRest::Missing   => ("MISSING".to_string(), false),
        KeyAtRest::Corrupt   => ("CORRUPT (unparseable key file)".to_string(), false),
    };
    let dr_escrow = if recovery_escrow {
        "recovery code set (off-node escrow; ADR-0026 slice A)".to_string()
    } else {
        "STUBBED (ADR-0026): no recovery escrow; key loss = node loss".to_string()
    };
```

Then in the returned `Status { .. }`, replace the hard-coded `key_at_rest` and `dr_escrow` lines and add `recovery_escrow`:

```rust
        keystore_ok,
        key_at_rest,
        runtime_role,
        db_floor_enforced: !can_insert,
        dr_escrow,
        recovery_escrow,
```

(Remove the old `key_at_rest: "PLAINTEXT...".into()` and `dr_escrow: "STUBBED...".into()` literals.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p cairn-node --test status 2>&1 | tail -20`
Expected: PASS (both status tests; plaintext path still reports PLAINTEXT/STUBBED and now `recovery_escrow=false`).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/identity.rs crates/cairn-node/tests/status.rs
git commit -m "feat(status): surface sealed at-rest posture + recovery escrow (ADR-0026 slice A)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: CLI wiring (main.rs)

**Files:**
- Modify: `crates/cairn-node/src/main.rs`

**Interfaces:**
- Consumes: `keystore::{generate_plaintext, generate_sealed, seal_existing, load, key_at_rest_state, KeyAtRest}`, `seal::generate_recovery_code` (Tasks 2-4).
- Produces: CLI behavior only (no library API). New `SealKey` subcommand; `Init` gains `--passphrase`/`--insecure-plaintext`.

- [ ] **Step 1: Add helpers + recovery-code printer**

In `crates/cairn-node/src/main.rs`, add near the top (after the imports):

```rust
/// Resolve the operational passphrase: `--passphrase` flag, else CAIRN_KEY_PASSPHRASE,
/// else an interactive no-echo prompt. Errors if none is available (we never write an
/// unsealed key implicitly — use --insecure-plaintext for that).
fn resolve_passphrase(flag: Option<String>) -> anyhow::Result<String> {
    if let Some(p) = flag.filter(|s| !s.is_empty()) {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("CAIRN_KEY_PASSPHRASE") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    let p = rpassword::prompt_password("operational passphrase: ")?;
    if p.is_empty() {
        anyhow::bail!("no passphrase provided (or use --insecure-plaintext)");
    }
    Ok(p)
}

/// Load the signing key for a runtime command. Uses CAIRN_KEY_PASSPHRASE; if the key
/// is sealed and the env var is absent, prompts. A plaintext key needs no secret.
fn load_signing_key(path: &std::path::Path) -> anyhow::Result<cairn_event::SigningKey> {
    use cairn_node::keystore::{key_at_rest_state, load, KeyAtRest};
    let secret = std::env::var("CAIRN_KEY_PASSPHRASE").ok().filter(|s| !s.is_empty());
    let secret = match secret {
        Some(s) => Some(s),
        None if matches!(key_at_rest_state(path), KeyAtRest::Sealed { .. }) => {
            Some(rpassword::prompt_password("operational passphrase: ")?)
        }
        None => None,
    };
    Ok(load(path, secret.as_deref())?)
}

/// Print a freshly-minted recovery code exactly once, with the honest loss warning.
fn print_recovery_code(code: &str) {
    eprintln!();
    eprintln!("=== RECOVERY CODE — shown ONCE. Write it down; store it OFF-SITE. ===");
    eprintln!("    {code}");
    eprintln!("=== This is the only off-node way to recover this node's signing key. ===");
    eprintln!("=== Lose BOTH this code and the passphrase and the node is permanently ===");
    eprintln!("=== lost — recoverable only by re-provisioning a new identity. ===");
    eprintln!();
}
```

- [ ] **Step 2: Extend the `Init` command and add `SealKey`**

In the `enum Cmd`, replace the `Init` variant and add `SealKey`:

```rust
    /// Provision this node: mint a keypair (SEALED by default) and append genesis.
    Init {
        #[arg(long)] name: String,
        #[arg(long)] address: String,
        /// Operational passphrase (else CAIRN_KEY_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")] passphrase: Option<String>,
        /// Write the key UNSEALED (test nodes only — no recovery escrow).
        #[arg(long)] insecure_plaintext: bool,
    },
    /// Seal an existing plaintext key file and mint a fresh recovery code.
    SealKey {
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")] passphrase: Option<String>,
    },
```

- [ ] **Step 3: Update the `Init` match arm and add the `SealKey` arm**

Replace the `Cmd::Init { name, address } => { .. }` arm with:

```rust
        Cmd::Init { name, address, passphrase, insecure_plaintext } => {
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let (sk, kid) = if insecure_plaintext {
                eprintln!("WARNING: --insecure-plaintext: signing key written UNSEALED (test use only)");
                cairn_node::keystore::generate_plaintext(&cli.key)?
            } else {
                let op = resolve_passphrase(passphrase)?;
                let code = cairn_node::seal::generate_recovery_code();
                let pair = cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?;
                print_recovery_code(&code);
                pair
            };
            let node_id = cairn_node::identity::provision(&db, &sk, &kid, &name, &address).await?;
            println!("provisioned node {node_id}\nfingerprint {}", cairn_event::short_fingerprint(&kid)?);
        }
        Cmd::SealKey { passphrase } => {
            let op = resolve_passphrase(passphrase)?;
            let code = cairn_node::seal::generate_recovery_code();
            cairn_node::keystore::seal_existing(&cli.key, &op, &code)?;
            println!("key at {} sealed.", cli.key.display());
            print_recovery_code(&code);
        }
```

- [ ] **Step 4: Route the key-loading commands through `load_signing_key`**

Replace each `let sk = cairn_node::keystore::load(&cli.key, None)?;` (in `PairOffer`, `PairAccept`, `Unpeer`, `Serve`, `Run` arms) with:

```rust
            let sk = load_signing_key(&cli.key)?;
```

- [ ] **Step 5: Print `recovery_escrow` in the `Status` arm**

In the `Cmd::Status` arm, after the `dr_escrow` print line, add:

```rust
            println!("recovery_esc  {}", st.recovery_escrow);
```

- [ ] **Step 6: Build + clippy**

Run: `cargo build -p cairn-node --all-targets 2>&1 | tail -20`
Expected: clean build.
Run: `cargo clippy -p cairn-node --all-targets 2>&1 | tail -20`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cli): sealed init + recovery code, seal-key migration, passphrase load (ADR-0026 slice A)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: DB-gated integration test for the sealed init + escrow path

**Files:**
- Create: `crates/cairn-node/tests/keystore_seal.rs`

**Interfaces:**
- Consumes: `db::{test_serial_guard, connect_and_load_schema}`, `keystore::{generate_sealed, load}`, `seal::generate_recovery_code`, `identity::{provision, status}` (existing + Tasks 2-4).

- [ ] **Step 1: Write the integration test**

Create `crates/cairn-node/tests/keystore_seal.rs`:

```rust
//! Integration: a SEALED node provisions, both recipients unseal the key, and
//! `status` reports the sealed posture + recovery escrow (ADR-0026 slice A).
//! DB-gated like the rest of the node suite; self-serializes via the advisory lock.
use cairn_node::{db, identity, keystore, seal};

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

#[tokio::test]
async fn sealed_init_produces_dual_recipient_key_and_surfaces_escrow() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let db = db::connect_and_load_schema(&base).await.unwrap();
    db.batch_execute("TRUNCATE node_event, local_node").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let kp = tmp.path().join("node.key");

    // Provision SEALED (the production path).
    let op = "correct horse battery staple";
    let code = seal::generate_recovery_code();
    let (sk, kid) = keystore::generate_sealed(&kp, op, &code).unwrap();
    identity::provision(&db, &sk, &kid, "A", "127.0.0.1:7900").await.unwrap();

    // Both recipients recover the same key; no/ wrong secret fails legibly (no panic).
    assert_eq!(keystore::load(&kp, Some(op)).unwrap().to_bytes(), sk.to_bytes());
    assert_eq!(keystore::load(&kp, Some(&code)).unwrap().to_bytes(), sk.to_bytes());
    assert!(keystore::load(&kp, None).is_err());
    assert!(keystore::load(&kp, Some("wrong")).is_err());

    // status reflects the sealed posture + escrow.
    let st = identity::status(&db, &kp).await.unwrap();
    assert!(st.keystore_ok);
    assert!(st.key_at_rest.contains("SEALED"), "got {:?}", st.key_at_rest);
    assert!(st.recovery_escrow, "sealed key must report an escrow");
    assert!(st.dr_escrow.contains("recovery code set"), "got {:?}", st.dr_escrow);
}
```

- [ ] **Step 2: Run it (with a DB) to verify it passes**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test keystore_seal 2>&1 | tail -20`
Expected: PASS if `CAIRN_TEST_PG` points at a Postgres with `cairn_pgx` installed; otherwise the test prints "skipped" and passes. (Confirm with the user which it is — see Task 7 verification.)

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/keystore_seal.rs
git commit -m "test(keystore): DB-gated sealed-init + escrow + status integration (ADR-0026 slice A)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Full-suite verification + docs

**Files:**
- Modify: `docs/HANDOVER.md` (node gaps section), `docs/ROADMAP.md` (Phase 5 at-rest seal line)

- [ ] **Step 1: Full workspace test + clippy + fmt**

Run: `cargo test --workspace 2>&1 | tail -30`
Expected: all pass (DB-gated tests skip cleanly without `CAIRN_TEST_PG`; if the user has a local PG + `cairn_pgx`, run with it set and confirm green).
Run: `cargo clippy --workspace --all-targets 2>&1 | tail -20` → no warnings.
Run: `cargo fmt --all -- --check` → clean (run `cargo fmt --all` if not).

- [ ] **Step 2: Update HANDOVER node gaps**

In `docs/HANDOVER.md`, under the federating-node "Honest gaps / follow-ons", update the at-rest line: the keystore is now SEALED (Argon2id + XChaCha20-Poly1305, dual-recipient) with an off-node recovery code minted at `init`; `key_at_rest` reports `SEALED`, `dr_escrow` reports `recovery code set`. Remaining DR work: the sealed *local-state export* (config + drafts), backup-as-cold-peer (slice B), and `supersede`/new-identity restore (slice C) are still open.

- [ ] **Step 3: Update ROADMAP Phase 5**

In `docs/ROADMAP.md` Phase 5, mark "At-rest seal — replace plaintext-0600 keystore" as done at the node level (ADR-0026 slice A: dual-recipient seal + recovery escrow), with the sealed local-state export / cold-peer / supersede slices noted as the remaining ADR-0026 work.

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(dr): record keystore seal + recovery escrow shipped (ADR-0026 slice A)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 5: Push + open PR**

```bash
git push -u origin harden-node-keystore-seal-adr0026
gh pr create --base main --title "Harden node: at-rest keystore seal + recovery escrow (ADR-0026 slice A)" --body "$(cat <<'EOF'
Implements ADR-0026 slice A: seal cairn-node's Ed25519 signing key at rest and mint an off-node recovery escrow at provisioning.

## What
- New pure `seal.rs`: random DEK seals the seed (XChaCha20-Poly1305); DEK wrapped twice under Argon2id KEKs — operational passphrase + one-time recovery code (dual-recipient). Magic-prefixed CBOR; tamper-evident by AEAD tag.
- `keystore.rs`: `generate_sealed`/`generate_plaintext`/`seal_existing`/auto-detect `load`/`key_at_rest_state`.
- CLI: `init` seals by default (`--insecure-plaintext` escape hatch), prints the recovery code once; new `seal-key` migration; daemon loads via `CAIRN_KEY_PASSPHRASE`/prompt.
- `status`: `key_at_rest SEALED`, `dr_escrow recovery code set`, new `recovery_escrow` field.

Closes the `key_at_rest PLAINTEXT` / `dr_escrow STUBBED` honest gaps.

## Honest non-goals (still open ADR-0026 work)
Sealed local-state export (config + drafts), backup-as-cold-peer (slice B), `supersede`/new-identity restore (slice C), Shamir M-of-N, QR, TPM. Loss ceiling (lose both passphrase AND recovery code => node loss) is documented, not engineered away.

## Testing
TDD throughout: seal/unseal roundtrip (both recipients), wrong-secret + per-field tamper, CBOR roundtrip, base32, keystore migration; DB-gated integration test for sealed init + escrow + status. Full workspace `cargo test` / `clippy` / `fmt` green.

Design: `docs/superpowers/specs/2026-06-24-node-keystore-seal-recovery-escrow-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**Spec coverage:** seal construction (Tasks 1-2) ✓; refuse silent plaintext (Task 5 init) ✓; dual-recipient unlock (Task 2 + Task 5 load) ✓; recovery code mint/display (Tasks 1, 5) ✓; auto-detect + migration (Task 3) ✓; status surfacing incl. `recovery_escrow` (Task 4) ✓; no DB/event change (escrow derived from file) ✓; deps + licenses (Task 1) ✓; tests incl. tamper + integration (Tasks 1-2, 6) ✓; non-goals recorded (Task 7) ✓.

**Placeholder scan:** none — every code step carries full code.

**Type consistency:** `generate_sealed/generate_plaintext/seal_existing/load/key_at_rest_state`, `KeyAtRest { Sealed{dual_recipient}, Plaintext, Missing, Corrupt }`, `seal::{seal, unseal, to_cbor, from_cbor, generate_recovery_code, normalize_recovery_code, base32_encode, base32_decode, SealedKey, Wrap, ArgonParams, SealError}`, `Status.recovery_escrow` — names used consistently across tasks. `load` keeps its existing `(path, Option<&str>)` signature so untouched callers compile; `generate_and_seal` fully removed and all call sites migrated in Task 3.
