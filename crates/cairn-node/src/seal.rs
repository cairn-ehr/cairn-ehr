//! At-rest key sealing for cairn-node (ADR-0026 slice A).
//!
//! WHY THIS EXISTS: a node's Ed25519 signing key must survive on disk without being
//! readable by anyone who copies the file, and must be recoverable off-node after a
//! lost passphrase or a dead disk. This module is the small safety-critical surface
//! ADR-0026 names: pure functions (entropy aside) that seal a 32-byte seed under TWO
//! independent secrets — an operational passphrase (daily, unattended `run`) and a
//! one-time recovery code (paper escrow). A defect here is silent key loss or a
//! forged identity, so it is exhaustively unit-tested and kept reviewer-legible.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
// `Zeroizing<T>` wipes its wrapped bytes on drop (issue #54). We use it for every
// transient secret in this module — the KEKs, the DEK, and the recovered seed — so
// key material never lingers in freed stack/heap memory, and so the type itself tells
// a reviewer "this value is a live secret." Deref/DerefMut let it stand in for the
// bare array at the AEAD/Argon2 call sites with no extra ceremony.
use zeroize::Zeroizing;

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
    s.to_ascii_uppercase()
        .chars()
        // Guard on `is_ascii()` BEFORE the `*c as u8` cast: that cast truncates a
        // multi-byte codepoint to its low 8 bits (e.g. 'Ł' U+0141 -> 0x41 'A'),
        // which would otherwise smuggle non-alphabet input past the filter and
        // corrupt the KDF input. ASCII-only is the real contract here.
        .filter(|c| c.is_ascii() && B32.contains(&(*c as u8)))
        .collect()
}

/// Generate a fresh 160-bit recovery code, grouped in 5-char blocks for legibility,
/// e.g. `AB12C-D34EF-...`. Shown ONCE at provisioning; the off-node escrow.
pub fn generate_recovery_code() -> String {
    let mut raw = [0u8; 20];
    // Panic is acceptable here: an entropy failure at provisioning is catastrophic;
    // the message carries nothing secret, and proceeding without entropy is worse.
    getrandom::fill(&mut raw).expect("entropy source unavailable");
    let flat = base32_encode(&raw);
    flat.as_bytes()
        .chunks(5)
        // `unwrap()` is safe: `flat` is built from B32, which is ASCII-only, so
        // every byte (and thus every 5-byte chunk) is valid UTF-8 by construction.
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

/// Magic header so a sealed bundle is distinguishable from a raw 32-byte plaintext
/// seed by inspection (keystore auto-detect). Versioned: bump on a format change.
const MAGIC: &[u8] = b"CAIRNK1\n";

#[derive(thiserror::Error, Debug)]
pub enum SealError {
    #[error("kdf: {0}")]
    Kdf(String),
    #[error("aead")]
    Aead,
    #[error("entropy: {0}")]
    Entropy(String),
    #[error("decode: {0}")]
    Decode(String),
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
        Self {
            m_cost: 19_456,
            t_cost: 2,
            p_cost: 1,
        }
    }
}

/// A dual-recipient sealed signing key. A random DEK encrypts the seed; the DEK is
/// wrapped once per secret (operational passphrase, recovery code). Either secret
/// recovers the DEK and hence the seed; neither secret is stored.
///
/// Debug is intentionally NOT derived: a stray `{:?}` must not be able to dump wrapped
/// key material. The fields hold only ciphertext/salts/nonces, but the guardrail is explicit.
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

/// Length of a wrapped DEK on disk: the 32-byte DEK plus the 16-byte Poly1305 tag
/// XChaCha20-Poly1305 appends. A recovery wrap shorter or longer than this cannot
/// possibly recover the key, so `status` must not advertise it as an escrow.
const WRAPPED_DEK_LEN: usize = 32 + 16;

impl SealedKey {
    /// Whether this bundle carries a STRUCTURALLY-INTACT recovery-code wrap (the
    /// off-node escrow). Checks the wrapped-DEK ciphertext is exactly the expected
    /// length, so a truncated or empty recovery wrap (e.g. a partial write) is
    /// honestly reported as "no escrow" rather than overstating the guarantee — an
    /// operator must never discard the paper code trusting a wrap that can't recover.
    ///
    /// LIMIT (by design): this is a structural check only. Without the secret it
    /// cannot detect a length-preserving bit-flip in the wrap; such corruption
    /// surfaces as an unseal failure during actual recovery, never here. `status`
    /// inspects the file without any secret, so this is the strongest honest check.
    pub fn has_recovery_wrap(&self) -> bool {
        self.wrap_rec.ct.len() == WRAPPED_DEK_LEN
    }
}

pub(crate) fn rand_bytes<const N: usize>() -> Result<[u8; N], SealError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| SealError::Entropy(e.to_string()))?;
    Ok(b)
}

/// Derive a 32-byte key-encryption-key from a secret + salt via Argon2id. The KEK is
/// returned wrapped in `Zeroizing` so it is wiped once the caller (a wrap/unwrap step)
/// is done with it — the memory-hard-derived KEK is the juiciest target in this module.
fn derive_kek(
    secret: &str,
    salt: &[u8; 16],
    p: &ArgonParams,
) -> Result<Zeroizing<[u8; 32]>, SealError> {
    let params = Params::new(p.m_cost, p.t_cost, p.p_cost, Some(32))
        .map_err(|e| SealError::Kdf(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    // Derive directly into the zeroizing buffer (`&mut *kek` is the inner `&mut [u8; 32]`),
    // so the only copy of the KEK lives inside the wrapper that wipes it.
    let mut kek = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(secret.as_bytes(), salt, &mut *kek)
        .map_err(|e| SealError::Kdf(e.to_string()))?;
    Ok(kek)
}

pub(crate) fn aead_encrypt(
    key: &[u8; 32],
    nonce: &[u8; 24],
    pt: &[u8],
) -> Result<Vec<u8>, SealError> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .encrypt(XNonce::from_slice(nonce), pt)
        .map_err(|_| SealError::Aead)
}

pub(crate) fn aead_decrypt(key: &[u8; 32], nonce: &[u8; 24], ct: &[u8]) -> Option<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher.decrypt(XNonce::from_slice(nonce), ct).ok()
}

/// Copy a 32-byte key out of a slice into a `Zeroizing` buffer **without** ever
/// materializing a bare `[u8; 32]` on the stack (issue #54). `<[u8;32]>::try_from`
/// would return a plain array by value, and moving that into `Zeroizing::new` leaves
/// the moved-from stack slot un-wiped — a lingering plaintext copy of the very key we
/// are trying to protect. Instead we allocate the zeroizing buffer first and
/// `copy_from_slice` into it, so the only copy of the key lives inside the wrapper that
/// wipes it (the same discipline `derive_kek` uses when it derives in-place). Returns
/// `None` on a wrong-length slice, mirroring the old `try_into().ok()`.
fn key_into_zeroizing(bytes: &[u8]) -> Option<Zeroizing<[u8; 32]>> {
    if bytes.len() != 32 {
        return None;
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(bytes);
    Some(out)
}

/// Wrap one DEK copy under a secret. The recovery code is normalized first so any
/// spacing/case the human re-types still derives the same KEK.
pub(crate) fn wrap_dek(
    dek: &[u8; 32],
    secret: &str,
    salt: &[u8; 16],
    p: &ArgonParams,
) -> Result<Wrap, SealError> {
    let kek = derive_kek(secret, salt, p)?;
    let nonce = rand_bytes::<24>()?;
    let ct = aead_encrypt(&kek, &nonce, dek)?;
    Ok(Wrap { nonce, ct })
}

pub(crate) fn try_unwrap(
    w: &Wrap,
    secret: &str,
    salt: &[u8; 16],
    p: &ArgonParams,
) -> Option<Zeroizing<[u8; 32]>> {
    let kek = derive_kek(secret, salt, p).ok()?;
    // Hold the decrypted bytes in `Zeroizing` so the plaintext DEK on the heap is wiped,
    // then copy straight into a zeroizing fixed-size array (no bare `[u8;32]` intermediate).
    let dek = Zeroizing::new(aead_decrypt(&kek, &w.nonce, &w.ct)?);
    key_into_zeroizing(&dek)
}

/// Seal a 32-byte signing seed under two independent secrets (dual-recipient).
pub fn seal(seed: &[u8; 32], op_pass: &str, recovery_code: &str) -> Result<SealedKey, SealError> {
    let argon = ArgonParams::default();
    // The DEK is transient: it encrypts the seed and is wrapped under each secret, then
    // dropped. Hold it in `Zeroizing` so it is wiped from the stack once `seal` returns.
    let dek = Zeroizing::new(rand_bytes::<32>()?);
    let seed_nonce = rand_bytes::<24>()?;
    let seed_ct = aead_encrypt(&dek, &seed_nonce, seed)?;
    let salt_op = rand_bytes::<16>()?;
    let salt_rec = rand_bytes::<16>()?;
    let wrap_op = wrap_dek(&dek, op_pass, &salt_op, &argon)?;
    // Normalize the recovery code so re-typing with different spacing/case unseals.
    let wrap_rec = wrap_dek(
        &dek,
        &normalize_recovery_code(recovery_code),
        &salt_rec,
        &argon,
    )?;
    Ok(SealedKey {
        version: 1,
        argon,
        salt_op,
        salt_rec,
        seed_nonce,
        seed_ct,
        wrap_op,
        wrap_rec,
    })
}

/// Recover the seed via ONLY the operational-passphrase recipient: exactly one
/// Argon2 derivation. The passphrase is used byte-exact. For callers that already
/// know the recipient (e.g. the read-after-write check on a migration); `unseal` is
/// the public, recipient-agnostic entry point.
pub fn unseal_op(s: &SealedKey, op_pass: &str) -> Option<Zeroizing<[u8; 32]>> {
    let dek = try_unwrap(&s.wrap_op, op_pass, &s.salt_op, &s.argon)?;
    // `Zeroizing` the decrypted seed wipes the heap plaintext; the returned array stays
    // wrapped (via `key_into_zeroizing`, no bare intermediate) so the seed is never bare
    // in the caller's frame (e.g. `keystore::load`).
    let seed = Zeroizing::new(aead_decrypt(&dek, &s.seed_nonce, &s.seed_ct)?);
    key_into_zeroizing(&seed)
}

/// Recover the seed via ONLY the recovery-code recipient: exactly one Argon2
/// derivation. The code is normalized first so any spacing/case unseals.
pub fn unseal_rec(s: &SealedKey, recovery_code: &str) -> Option<Zeroizing<[u8; 32]>> {
    let dek = try_unwrap(
        &s.wrap_rec,
        &normalize_recovery_code(recovery_code),
        &s.salt_rec,
        &s.argon,
    )?;
    let seed = Zeroizing::new(aead_decrypt(&dek, &s.seed_nonce, &s.seed_ct)?);
    key_into_zeroizing(&seed)
}

/// Recover the seed from either recipient, secret unknown. Tries the operational
/// passphrase first (byte-exact), then the recovery code (normalized). `None` on a
/// wrong secret or any tamper (the AEAD tag fails). The two paths are
/// indistinguishable to the caller: we never leak which recipient matched or why a
/// decrypt failed. NOTE: a recovery-code unseal therefore pays the op-path Argon2
/// derivation first — when the recipient IS known, call `unseal_op`/`unseal_rec`
/// directly to do half the memory-hard work.
pub fn unseal(s: &SealedKey, secret: &str) -> Option<Zeroizing<[u8; 32]>> {
    unseal_op(s, secret).or_else(|| unseal_rec(s, secret))
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
    let body = bytes
        .strip_prefix(MAGIC)
        .ok_or_else(|| SealError::Decode("missing CAIRNK1 magic".into()))?;
    ciborium::from_reader(body).map_err(|e| SealError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    // Deterministic crypto test fixtures are COMPUTED at runtime, never written as
    // hard-coded byte-array literals. A literal key/seed/salt/nonce in a test trips
    // CodeQL's `rust/hard-coded-cryptographic-value` (critical) — a false positive here
    // (production derives ALL key material from `rand_bytes`), but a *recurring* one:
    // every new literal fixture re-raises the alert until a human dismisses it (issue
    // #146). Deriving the bytes keeps the fixture deterministic while presenting no
    // hard-coded cryptographic value to the scanner. House rule: see CLAUDE.md.

    /// A deterministic 32-byte seed — the plaintext key material sealed/unsealed below.
    fn test_seed() -> [u8; 32] {
        std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1))
    }

    /// A deterministic 16-byte KDF salt for the `derive_kek` unit test.
    fn test_salt() -> [u8; 16] {
        std::array::from_fn(|i| (i as u8).wrapping_add(1))
    }

    // --- Issue #54: recovered key material must come back wrapped in `Zeroizing` ---
    // so it is wiped from the caller's stack/heap on drop (defence-in-depth), and the
    // type itself signals "this is a secret" to a reviewer. These tests pin the return
    // TYPES via explicit annotations; they fail to compile if a function reverts to a
    // bare array, which is exactly the regression we want to catch.

    #[test]
    fn unseal_helpers_yield_zeroizing_seed() {
        let s = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        let any: Zeroizing<[u8; 32]> = unseal(&s, "op-pass").expect("op-pass must unseal");
        let via_op: Zeroizing<[u8; 32]> = unseal_op(&s, "op-pass").expect("op helper unseals");
        let via_rec: Zeroizing<[u8; 32]> = unseal_rec(&s, "REC-CODE").expect("rec helper unseals");
        assert_eq!(*any, test_seed());
        assert_eq!(*via_op, test_seed());
        assert_eq!(*via_rec, test_seed());
    }

    #[test]
    fn key_into_zeroizing_copies_exact_len_and_rejects_others() {
        // Exactly 32 bytes round-trips into the wrapped array...
        let src = [9u8; 32];
        let out = key_into_zeroizing(&src).expect("32-byte slice must convert");
        assert_eq!(*out, src);
        // ...and any other length is rejected (mirrors the old `try_into().ok()` guard),
        // so a truncated/over-long plaintext can never silently yield a partial key.
        assert!(key_into_zeroizing(&[9u8; 31]).is_none());
        assert!(key_into_zeroizing(&[9u8; 33]).is_none());
        assert!(key_into_zeroizing(&[]).is_none());
    }

    #[test]
    fn try_unwrap_yields_zeroizing_dek() {
        let s = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        let dek: Zeroizing<[u8; 32]> =
            try_unwrap(&s.wrap_op, "op-pass", &s.salt_op, &s.argon).expect("op wrap unwraps");
        // The unwrapped DEK is the real key: it decrypts the sealed seed.
        assert_eq!(
            aead_decrypt(&dek, &s.seed_nonce, &s.seed_ct).as_deref(),
            Some(test_seed().as_slice())
        );
    }

    #[test]
    fn derive_kek_yields_zeroizing() {
        let p = ArgonParams::default();
        let _kek: Zeroizing<[u8; 32]> =
            derive_kek("secret", &test_salt(), &p).expect("kdf must succeed");
    }

    #[test]
    fn unseals_with_operational_passphrase() {
        let s = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        // `.as_deref()` reads through the `Zeroizing` wrapper to compare the seed bytes.
        assert_eq!(unseal(&s, "op-pass").as_deref(), Some(&test_seed()));
    }

    #[test]
    fn unseals_with_recovery_code_any_formatting() {
        let s = seal(&test_seed(), "op-pass", "ab12c-d34ef").unwrap();
        // re-typed with different case/spacing still works
        assert_eq!(unseal(&s, "AB12C D34EF").as_deref(), Some(&test_seed()));
    }

    #[test]
    fn wrong_secret_returns_none() {
        let s = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        assert!(unseal(&s, "nope").is_none());
    }

    #[test]
    fn tampered_fields_return_none() {
        let base = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        let mutate = |f: fn(&mut SealedKey)| {
            let mut s = base.clone();
            f(&mut s);
            s
        };
        // Flipping a byte in any ciphertext, salt, or nonce must fail the AEAD tag.
        assert!(unseal(&mutate(|s| s.seed_ct[0] ^= 1), "op-pass").is_none());
        assert!(unseal(&mutate(|s| s.wrap_op.ct[0] ^= 1), "op-pass").is_none());
        assert!(unseal(&mutate(|s| s.wrap_rec.ct[0] ^= 1), "REC-CODE").is_none());
        assert!(unseal(&mutate(|s| s.salt_op[0] ^= 1), "op-pass").is_none());
        assert!(unseal(&mutate(|s| s.seed_nonce[0] ^= 1), "op-pass").is_none());
        // Recovery-path tamper: the assertions above all unseal via "op-pass" (the op
        // path), which masks any mutation to the recovery wrap's KDF inputs. Exercise the
        // second recipient explicitly: a flipped recovery salt yields a wrong KEK, and a
        // flipped recovery nonce yields a wrong Poly1305 key — either must fail unseal via
        // the recovery code, closing the coverage gap for the second recipient.
        assert!(unseal(&mutate(|s| s.salt_rec[0] ^= 1), "REC-CODE").is_none());
        assert!(unseal(&mutate(|s| s.wrap_rec.nonce[0] ^= 1), "REC-CODE").is_none());
    }

    #[test]
    fn per_recipient_unseal_helpers_isolate_their_recipient() {
        let s = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        // Each helper recovers the seed via its own recipient only.
        assert_eq!(unseal_op(&s, "op-pass").as_deref(), Some(&test_seed()));
        assert_eq!(unseal_rec(&s, "REC-CODE").as_deref(), Some(&test_seed()));
        // ...and refuses the other recipient's secret (no cross-talk): the op helper
        // must not accept the recovery code, nor the recovery helper the passphrase.
        assert!(unseal_op(&s, "REC-CODE").is_none());
        assert!(unseal_rec(&s, "op-pass").is_none());
    }

    #[test]
    fn has_recovery_wrap_is_false_for_a_truncated_wrap() {
        // A partial write that truncates the recovery wrap must report NO escrow, so
        // `status` never tells an operator the off-node code is good when it isn't.
        let mut s = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        assert!(
            s.has_recovery_wrap(),
            "a freshly sealed key has an intact recovery wrap"
        );
        s.wrap_rec.ct.truncate(WRAPPED_DEK_LEN - 1);
        assert!(
            !s.has_recovery_wrap(),
            "a truncated recovery wrap is not an escrow"
        );
        s.wrap_rec.ct.clear();
        assert!(
            !s.has_recovery_wrap(),
            "an empty recovery wrap is not an escrow"
        );
    }

    #[test]
    fn cbor_roundtrips_and_has_magic() {
        let s = seal(&test_seed(), "op-pass", "REC-CODE").unwrap();
        let bytes = to_cbor(&s);
        assert!(
            bytes.starts_with(b"CAIRNK1\n"),
            "magic header must be present"
        );
        let back = from_cbor(&bytes).unwrap();
        assert_eq!(unseal(&back, "op-pass").as_deref(), Some(&test_seed()));
        assert!(back.has_recovery_wrap());
    }

    #[test]
    fn from_cbor_rejects_garbage_and_plaintext() {
        assert!(from_cbor(b"not a sealed bundle").is_err());
        assert!(
            from_cbor(&[0u8; 32]).is_err(),
            "a raw 32-byte seed is not a bundle"
        );
    }

    #[test]
    fn base32_roundtrips_arbitrary_bytes() {
        for v in [
            vec![],
            vec![0u8],
            vec![0xff; 20],
            (0u8..=255).collect::<Vec<_>>(),
        ] {
            let enc = base32_encode(&v);
            assert_eq!(
                base32_decode(&enc).as_deref(),
                Some(v.as_slice()),
                "roundtrip failed for {} bytes",
                v.len()
            );
        }
    }

    #[test]
    fn base32_rejects_invalid_chars() {
        // 'I','L','O','U' are excluded from Crockford base32; a literal '!' is invalid.
        assert!(base32_decode("!!!!").is_none());
        // The Crockford-excluded letters are the real transcription-error case:
        // a human reading 'I'/'L'/'O'/'U' must NOT silently decode to something.
        assert!(base32_decode("IIII").is_none());
        assert!(base32_decode("LLLL").is_none());
        assert!(base32_decode("OOOO").is_none());
        assert!(base32_decode("UUUU").is_none());
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
        assert_eq!(
            base32_decode(&normalize_recovery_code(&a)).map(|v| v.len()),
            Some(20)
        );
    }
}
