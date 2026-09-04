//! The custody plane's two key types — issue #511.
//!
//! WHY THIS EXISTS. Every key here used to be a bare `[u8; 32]`: the X25519 secret half, its
//! public half, the node's Ed25519 signing seed and a per-event DEK, all the same type. So
//! installing a PUBLIC half as this node's SECRET custody key compiled — and `node_unwrap_key`
//! is a **singleton** whose registrar then refuses the real key forever. That is the #495
//! failure shape (a restored solo clinic that can open none of its own record) reintroduced one
//! layer up, and none of the runtime defences could catch it: the read-after-write checks in
//! `keystore::generate_unwrap_sealed` and `CustodyKeyDestination::install` prove the file holds
//! *the bytes we wrote*, never *a key that opens anything*.
//!
//! WHAT THESE TWO TYPES DO — AND DO NOT DO. Secret-vs-public is now a COMPILE error everywhere.
//! **Secret-vs-secret is not.** An unwrap secret, an Ed25519 signing seed and a DEK are all
//! [`Secret32`], so `Secret32::from_bytes(sk.to_bytes())` still compiles. What changed is that
//! it stopped being an implicit coercion a reviewer cannot see and became a **named, greppable
//! line** — and in production there is exactly one:
//! `cairn_keystore::keystore::adopt_derived_unwrap_secret`, the ADR-0066 adoption migration,
//! pinned by `cairn-node/tests/unwrap_secret_is_not_derived.rs`.
//! `keystore::unwrap_secret_is_the_signing_seed` remains the only defence against the
//! signing-seed-as-unwrap-secret confusion, and it is wired only where both keys are genuinely
//! this node's. This boundary — one wrapper, not one per role — is the same one
//! [`crate::VerifiedKid`] draws in `contributor.rs`, and it was chosen deliberately over
//! role-typed newtypes (`UnwrapSecret`/`SigningSeed`/`Dek`).
//!
//! ## The three lines #511 reported, recorded as compile-fail doctests
//!
//! Rust has no `#[should_not_compile]`, and this tree carries no `trybuild` dependency, so the
//! defect is pinned as doctests instead — they FAIL the suite if the code inside them ever
//! starts compiling again. Key material in them is DERIVED, not a literal, for the same house
//! rule 6 reason it is everywhere else: a byte-array literal in a crypto position is what trips
//! CodeQL's `rust/hard-coded-cryptographic-value`, and a rule that is followed everywhere
//! except in the documentation is a rule a reader will reasonably conclude is optional.
//!
//! 1. The PUBLIC half installed as this node's SECRET custody key:
//!
//! ```compile_fail
//! use cairn_event::keys::Secret32;
//! fn install(_: &Secret32) {}
//! let secret = Secret32::from_bytes(std::array::from_fn(|i| (i as u8).wrapping_mul(7)));
//! let public = cairn_event::seal::unwrap_public(&secret);
//! install(&public);
//! ```
//!
//! 2. The PUBLIC half exported where the secret belongs:
//!
//! ```compile_fail
//! use cairn_event::keys::Secret32;
//! let secret = Secret32::from_bytes(std::array::from_fn(|i| (i as u8).wrapping_mul(7)));
//! let public = cairn_event::seal::unwrap_public(&secret);
//! let _wrapped = cairn_event::seal::unwrap_dek(&vec![0u8; 104], &public);
//! ```
//!
//! 3. A DEK wrapped TO a secret rather than to a public half:
//!
//! ```compile_fail
//! use cairn_event::keys::Secret32;
//! let dek = Secret32::from_bytes(std::array::from_fn(|i| (i as u8).wrapping_mul(3)));
//! let recipient = Secret32::from_bytes(std::array::from_fn(|i| (i as u8).wrapping_mul(5)));
//! let _ = cairn_event::seal::wrap_dek_for(&dek, &recipient);
//! ```
//!
//! And the POSITIVE CONTROL, without which the three above guard nothing — a `compile_fail`
//! block that fails for the wrong reason (a typo, a missing import, a moved path) passes
//! silently:
//!
//! ```
//! use cairn_event::keys::Secret32;
//! let secret = Secret32::from_bytes(std::array::from_fn(|i| (i as u8).wrapping_mul(7)));
//! let public = cairn_event::seal::unwrap_public(&secret);
//! let wrapped = cairn_event::seal::wrap_dek_for(&secret, &public).unwrap();
//! assert_eq!(cairn_event::seal::unwrap_dek(&wrapped, &secret).unwrap(), secret);
//! ```
//!
//! WHY A SEPARATE FILE. `seal.rs` is where these are USED, and it re-exports them so
//! `cairn_event::seal::Secret32` — the path #511 names and every call site uses — resolves. But
//! `seal.rs` is already past the project's 500-line guideline, so the definitions live here.
//! One definition, two paths, the second an ordinary Rust re-export.

use serde::de::{Error as DeError, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// The length every key in this plane has. Named so the serde impls, the constructors and the
/// refusal messages cannot drift apart from one another.
const KEY_LEN: usize = 32;

/// 32 bytes of SECRET key material: an X25519 unwrap secret, an Ed25519 signing seed, a
/// per-event DEK, or a wrap KEK. Wiped on drop; `Debug` redacts; equality is constant-time.
///
/// Deliberately **not** `Copy`: a `Copy` secret leaves unwiped duplicates by construction,
/// which is the opposite of what the `Zeroizing` inner buys. `Clone` is fine — every clone is
/// itself wiped on drop — and is what callers use when they must hand one on by value.
#[derive(Clone)]
pub struct Secret32(Zeroizing<[u8; KEY_LEN]>);

impl Secret32 {
    /// An all-zero secret, to be filled in place via [`Self::as_mut_bytes`].
    ///
    /// This exists so the crate can keep its "derive directly into the zeroizing buffer" idiom:
    /// HKDF output, CSPRNG output and AEAD plaintext are written straight into the wiped
    /// buffer, so the material never exists as a bare array on the stack that nothing can reach
    /// to wipe (issue #54).
    pub fn zeroed() -> Self {
        Secret32(Zeroizing::new([0u8; KEY_LEN]))
    }

    /// The ONE raw entry point, and deliberately a named function rather than a `From` impl.
    ///
    /// Every place a loose 32 bytes becomes a secret should be findable with one grep, because
    /// that is exactly where a signing seed can be misfiled as an unwrap secret (see the module
    /// header). A `From<[u8; 32]>` would make those conversions invisible at the call site,
    /// which is the property this whole type exists to remove.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Secret32(Zeroizing::new(bytes))
    }

    /// Copy a slice into a pre-zeroed buffer, or refuse.
    ///
    /// `None` on any length but 32. A wrong-length key is corruption or a version mismatch,
    /// never something to pad or truncate — and it arrives this way whenever the length is not
    /// statically known: off a database column, out of a decoded bundle, over a wire field.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != KEY_LEN {
            return None;
        }
        let mut out = Self::zeroed();
        out.as_mut_bytes().copy_from_slice(bytes);
        Some(out)
    }

    /// The escape hatch, for feeding crypto primitives that take a raw array — AEAD keys,
    /// `StaticSecret::from`, HKDF input keying material.
    ///
    /// **Not for logging.** Use `{:?}`, which redacts.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Fill in place — see [`Self::zeroed`].
    pub fn as_mut_bytes(&mut self) -> &mut [u8; KEY_LEN] {
        &mut self.0
    }
}

/// Redacting rather than absent, and the difference is the point.
///
/// An absent `Debug` is a compile error at each site that wants one, and the workaround a
/// future author reaches for — an accessor plus `hex::encode` — is strictly worse than a
/// redaction. A redacting `Debug` is a POSITIVE guarantee at every present and future site: it
/// makes `#[derive(Debug)]` on a containing type safe, which is how `localstate::LocalState`
/// stopped needing a hand-written `Debug` whose only job was redacting this one field. Deviates
/// from #511's literal "never Debug" for that reason.
impl std::fmt::Debug for Secret32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret32(<redacted>)")
    }
}

/// Constant-time, unlike the `*a != *b` array comparisons this type replaces.
///
/// The threat is thin here — these comparisons are local and an attacker does not choose the
/// operands — but a key-equality operator that short-circuits on the first differing byte is
/// the kind of thing that becomes load-bearing later, in a place nobody re-reads. `subtle` is
/// already in this tree via the dalek crates; this only promotes it to a direct dependency.
impl PartialEq for Secret32 {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&*other.0).into()
    }
}
impl Eq for Secret32 {}

/// WIRE-CRITICAL — do not "tidy" this into `#[derive(Serialize)]`.
///
/// `LocalState::unwrap_secret` was `Option<Vec<u8>>`, and ciborium encodes a `Vec<u8>` as a
/// CBOR **array of unsigned ints** (`serde_bytes` is not in play for that field), not as a byte
/// string. This emits exactly that, so every `CAIRNL1` export written before #511 still
/// restores and every one written after is still readable by a build that predates these types.
/// Pinned end-to-end by `cairn-node/tests/localstate_wire_pins.rs`.
impl Serialize for Secret32 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter())
    }
}

/// Decodes into a PRE-SIZED ZEROIZING buffer — never into a `Vec<u8>` that would leave an
/// unwiped copy of the node's custody key in freed heap (#508's shape, narrowed here).
///
/// Refuses any length but 32 **at the parse boundary**, in words an operator can act on. That
/// is the refusal `localstate::recovered_unwrap_secret` used to make one layer later, moved to
/// where a corrupt bundle actually arrives — before anything is written to the keystore or
/// registered in the singleton `node_unwrap_key`.
impl<'de> Deserialize<'de> for Secret32 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl<'de> Visitor<'de> for KeyVisitor {
            type Value = Secret32;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "exactly {KEY_LEN} bytes of key material")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Secret32, A::Error> {
                let mut out = Secret32::zeroed();
                for (i, slot) in out.as_mut_bytes().iter_mut().enumerate() {
                    *slot = seq.next_element::<u8>()?.ok_or_else(|| {
                        A::Error::custom(format!(
                            "this key is {i} bytes, not {KEY_LEN} — refusing a key that cannot \
                             open this node's custody"
                        ))
                    })?;
                }
                if seq.next_element::<u8>()?.is_some() {
                    return Err(A::Error::custom(format!(
                        "this key is longer than {KEY_LEN} bytes — refusing a key that cannot \
                         open this node's custody"
                    )));
                }
                Ok(out)
            }
        }
        d.deserialize_seq(KeyVisitor)
    }
}

/// 32 bytes of PUBLIC key material — an X25519 unwrap public half.
///
/// Safe to log, store and register: it is published by design (it sits in the `node_unwrap_key`
/// table and travels on the wire inside the node-signed unwrap certificate), and it alone can
/// never unwrap anything. `Copy` is right here for the same reason it is wrong on [`Secret32`]:
/// nothing needs wiping.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PublicKey32([u8; KEY_LEN]);

impl PublicKey32 {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        PublicKey32(bytes)
    }

    /// `None` on any length but 32 — the shape a public half takes coming off a database column
    /// or a wire field, where the length is not statically known.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let arr: [u8; KEY_LEN] = bytes.try_into().ok()?;
        Some(PublicKey32(arr))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    pub fn to_bytes(self) -> [u8; KEY_LEN] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// House rule 6(a): derived at runtime, never a byte-array literal in a crypto context.
    /// `lineage` is a discriminator and is deliberately NOT called `salt` — CodeQL picks its
    /// sink by the NAME a value flows into, and a discriminator wearing a KDF's name mints a
    /// critical alert per call site (#527).
    fn bytes_fixture(lineage: u8) -> [u8; 32] {
        std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(lineage))
    }

    #[test]
    fn a_secret_round_trips_through_its_bytes() {
        let raw = bytes_fixture(1);
        assert_eq!(Secret32::from_bytes(raw).as_bytes(), &raw);
    }

    #[test]
    fn from_slice_refuses_any_length_but_32() {
        assert!(Secret32::from_slice(&bytes_fixture(1)[..31]).is_none());
        assert!(Secret32::from_slice(&[0u8; 33]).is_none());
        assert!(Secret32::from_slice(&[]).is_none());
        assert!(Secret32::from_slice(&bytes_fixture(1)).is_some());
    }

    #[test]
    fn a_zeroed_secret_is_all_zero_and_fillable_in_place() {
        let mut s = Secret32::zeroed();
        assert_eq!(s.as_bytes(), &[0u8; 32]);
        s.as_mut_bytes().copy_from_slice(&bytes_fixture(2));
        assert_eq!(s.as_bytes(), &bytes_fixture(2));
    }

    /// The whole point of the redaction: `{:?}` on a secret — or on anything containing one —
    /// must not put key material in a log line, a panic message, or an assert failure.
    #[test]
    fn debug_redacts_the_bytes() {
        let s = Secret32::from_bytes(bytes_fixture(3));
        let shown = format!("{s:?}");
        assert_eq!(shown, "Secret32(<redacted>)");
        assert!(
            !shown.contains(&hex::encode(bytes_fixture(3))[..8]),
            "Debug leaked key material: {shown}"
        );
    }

    /// A public key is published by design (it sits in `node_unwrap_key` and travels in the
    /// node-signed unwrap certificate), so its `Debug` shows the real bytes. The type IS the
    /// argument that printing it is safe.
    #[test]
    fn a_public_key_debug_shows_its_bytes() {
        let raw = bytes_fixture(4);
        let shown = format!("{:?}", PublicKey32::from_bytes(raw));
        assert!(
            shown.contains(&format!("{}", raw[1])),
            "a public half must be printable in full: {shown}"
        );
    }

    #[test]
    fn secrets_compare_by_value() {
        let a = Secret32::from_bytes(bytes_fixture(5));
        let b = Secret32::from_bytes(bytes_fixture(5));
        let c = Secret32::from_bytes(bytes_fixture(6));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// A one-byte difference in the LAST position must still be unequal. A constant-time
    /// comparison that accidentally stopped early would pass the previous test and fail this
    /// one, which is why both exist.
    #[test]
    fn secrets_differing_only_in_the_last_byte_are_unequal() {
        let a = Secret32::from_bytes(bytes_fixture(7));
        let mut b = a.clone();
        b.as_mut_bytes()[31] ^= 1;
        assert_ne!(a, b);
    }

    /// THE WIRE CONTRACT. `LocalState::unwrap_secret` was `Option<Vec<u8>>` and becomes
    /// `Option<Secret32>`; ciborium encodes a `Vec<u8>` as a CBOR ARRAY of unsigned ints
    /// (serde_bytes is not in play for that field), so `Secret32` must encode identically or
    /// every existing off-site `CAIRNL1` export stops restoring.
    /// `cairn-node/tests/localstate_wire_pins.rs` is the end-to-end proof; this is the
    /// unit-level one, and it fails first and far more legibly.
    #[test]
    fn a_secret_encodes_exactly_as_the_vec_it_replaces() {
        let raw = bytes_fixture(8);
        let mut as_vec = Vec::new();
        ciborium::into_writer(&raw.to_vec(), &mut as_vec).expect("Vec<u8> encodes");
        let mut as_secret = Vec::new();
        ciborium::into_writer(&Secret32::from_bytes(raw), &mut as_secret)
            .expect("Secret32 encodes");
        assert_eq!(
            hex::encode(&as_secret),
            hex::encode(&as_vec),
            "Secret32's CBOR must be byte-identical to the Vec<u8> it replaces in CAIRNL1"
        );
    }

    #[test]
    fn a_secret_decodes_from_the_vec_encoding_it_replaces() {
        let raw = bytes_fixture(9);
        let mut as_vec = Vec::new();
        ciborium::into_writer(&raw.to_vec(), &mut as_vec).expect("Vec<u8> encodes");
        let back: Secret32 = ciborium::from_reader(&as_vec[..]).expect("must decode");
        assert_eq!(back.as_bytes(), &raw);
    }

    /// A wrong-length slot must be refused AT THE PARSE BOUNDARY, in words an operator can
    /// act on — the refusal `localstate::recovered_unwrap_secret` used to make one layer
    /// later, moved to where a corrupt bundle actually arrives.
    #[test]
    fn a_short_secret_is_refused_with_a_legible_cause() {
        let mut short = Vec::new();
        ciborium::into_writer(&vec![0u8; 31], &mut short).expect("encodes");
        let err =
            ciborium::from_reader::<Secret32, _>(&short[..]).expect_err("31 bytes cannot be a key");
        let text = format!("{err}");
        assert!(
            text.contains("32"),
            "the refusal must name the expected length; got: {text}"
        );
    }

    #[test]
    fn a_long_secret_is_refused_with_a_legible_cause() {
        let mut long = Vec::new();
        ciborium::into_writer(&vec![0u8; 33], &mut long).expect("encodes");
        let err =
            ciborium::from_reader::<Secret32, _>(&long[..]).expect_err("33 bytes cannot be a key");
        assert!(
            format!("{err}").contains("32"),
            "the refusal must name the expected length"
        );
    }

    /// `PublicKey32` also lives in serialized shapes (`CustodyAdmission`, and any future slot),
    /// so its encoding is pinned against the `[u8; 32]` it replaces for the same reason.
    #[test]
    fn a_public_key_encodes_exactly_as_the_array_it_replaces() {
        let raw = bytes_fixture(10);
        let mut as_array = Vec::new();
        ciborium::into_writer(&raw, &mut as_array).expect("array encodes");
        let mut as_public = Vec::new();
        ciborium::into_writer(&PublicKey32::from_bytes(raw), &mut as_public)
            .expect("PublicKey32 encodes");
        assert_eq!(hex::encode(&as_public), hex::encode(&as_array));
    }

    #[test]
    fn a_public_key_from_slice_refuses_any_length_but_32() {
        assert!(PublicKey32::from_slice(&bytes_fixture(1)[..31]).is_none());
        assert!(PublicKey32::from_slice(&[0u8; 33]).is_none());
        assert!(PublicKey32::from_slice(&bytes_fixture(1)).is_some());
    }
}
