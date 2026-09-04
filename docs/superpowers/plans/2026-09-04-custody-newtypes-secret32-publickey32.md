# Custody newtypes — `Secret32` / `PublicKey32` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make installing an X25519 *public* half as this node's *secret* custody key a **compile
error**, by giving the custody plane two newtypes (`Secret32`, `PublicKey32`) instead of one bare
`[u8; 32]`, and close `LocalState`'s open producer set while doing it.

**Architecture:** Two types in a new `crates/cairn-event/src/keys.rs`, re-exported from
`cairn_event::seal`. Every function in the custody plane — across `cairn-event`, `cairn-keystore`,
`cairn-node`, `cairn-sync` and the separately-`exclude`d `extensions/cairn_pgx` tree — takes and
returns them instead of `[u8; 32]` / `Zeroizing<[u8; 32]>`. Crypto primitives (AEAD key/nonce
arguments) stay on `&[u8; 32]`, reached through `as_bytes()`. The one serialized field that changes
type (`LocalState::unwrap_secret`, a `CAIRNL1` slot) is protected by golden CBOR bytes frozen
**before** the change.

**Tech Stack:** Rust 1.96.0 (pinned in `rust-toolchain.toml`), `zeroize`, `subtle` (promoted from
transitive to direct in `cairn-event`), `ciborium` 0.2, `x25519-dalek`, `ed25519-dalek`,
`chacha20poly1305`.

**Spec:** [`docs/superpowers/specs/2026-09-04-custody-newtypes-secret32-publickey32-design.md`](../specs/2026-09-04-custody-newtypes-secret32-publickey32-design.md)

**Issue:** [#511](https://github.com/cairn-ehr/cairn-ehr/issues/511)

Paper-parity: not clinical-surface — this slice changes only the Rust type of key material already
moving through the custody plane. It adds, removes and reorders no human act at any layer, exposes no
clinician-reachable surface, and produces byte-identical `CAIRNL1` exports (pinned in Task 1). §1.2's
time/steps/cognitive-load benchmark has no workflow to measure here.

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible, checked before adding. `subtle` 2.6.1 is
  BSD-3-Clause and already in `Cargo.lock` via the dalek crates — this promotes it to a direct
  dependency of `cairn-event`, adding no new supply-chain surface.
- **TDD** — failing test first, then the code that makes it pass. Load-bearing here: this is §9
  safety-critical surface (a defect silently orphans a restored clinic's whole record).
- **House rule 6** — never hard-code cryptographic material in tests, and never give a
  non-cryptographic value a cryptographic NAME. Test key material is derived at runtime
  (`std::array::from_fn(|i| …)`); a discriminator is a `lineage`/`variant`/`seed`, never a
  `salt`/`nonce`/`iv`. The one exception in this plan is Task 1's **golden CBOR bytes**, which are a
  wire-format pin, not key material — the secret *inside* them is still runtime-derived.
- **Three cargo trees, three lockfiles.** `extensions/cairn_pgx` and `cairn-gui` are `exclude`d from
  the root workspace but ship anyway, depending on root crates **by path**. No root-workspace gate
  sees a stale sibling lockfile — only the `--locked` clippy run on those two trees does.
- **DB-free runs need `CAIRN_ALLOW_DB_SKIP=1`** (#450), or `cargo test` fails.
- **Never `git checkout -- <file>`** to undo an edit — it discards all uncommitted work in that file,
  unrecoverably. Copy to the scratchpad and `cp` back instead.
- **Never `cargo test | tail`** — the pipe masks cargo's exit code.
- Files aim under 500 lines (a guideline, not a cap). `seal.rs` is already 640, which is why the new
  types get their own file.

## File Structure

**Create**

| file | responsibility |
|---|---|
| `crates/cairn-event/src/keys.rs` | `Secret32` + `PublicKey32`: the two types, their constructors, accessors, redacting `Debug`, constant-time `PartialEq`, and the hand-written wire-compatible serde impls. Plus their unit tests. |
| `crates/cairn-node/tests/localstate_wire_pins.rs` | Golden CBOR bytes for a populated `LocalState`, frozen before the type change and required to stay green through it. |

**Modify**

| file | change |
|---|---|
| `crates/cairn-event/Cargo.toml` | add `subtle` |
| `crates/cairn-event/src/lib.rs` | `pub mod keys;`; cert fns take/return `PublicKey32` |
| `crates/cairn-event/src/seal.rs` | re-export the types; migrate every public signature |
| `crates/cairn-keystore/src/seal.rs` | `seal`/`unseal*` on `Secret32` |
| `crates/cairn-keystore/src/keystore.rs` | every unwrap-key fn on `Secret32`/`PublicKey32` |
| `crates/cairn-node/src/localstate.rs` | `unwrap_secret: Option<Secret32>`; closed producer set; derived `Debug`; widened `Drop`; `LskWraps`/`SealedLocalState` constructors |
| `crates/cairn-node/src/localstate_read.rs` | `Option<&Secret32>`; calls `LocalState::from_custody` |
| `crates/cairn-node/src/main.rs` | call sites |
| `crates/cairn-node/src/medication/sealed_submit.rs` | DEK return type |
| `crates/cairn-sync/src/unwrap_key.rs` | `FileOutcome`/`Resolution`/`resolve`/`registered_from_row` |
| `crates/cairn-sync/src/main.rs` | call sites; `CustodyAdmission::Grant` |
| `extensions/cairn_pgx/src/lib.rs` | call sites (separate cargo tree) |
| `crates/cairn-node/tests/*.rs`, `crates/cairn-sync/tests/*.rs` | fixtures and assertions |
| `Cargo.lock`, `extensions/cairn_pgx/Cargo.lock`, `cairn-gui/Cargo.lock` | refreshed |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | session state |

---

### Task 1: Freeze the `CAIRNL1` wire bytes before anything moves

**Files:**
- Test: `crates/cairn-node/tests/localstate_wire_pins.rs` (create)

**Interfaces:**
- Consumes: today's `cairn_node::localstate::{LocalState, to_cbor, from_cbor, EpisodeDek, episode_dek_to_cbor}`.
- Produces: two frozen constants — `POPULATED_BUNDLE_CBOR_HEX` and the fixture that builds the bundle
  it encodes — that Tasks 5 and 6 must keep green without editing.

**Why this is first.** `LocalState::unwrap_secret` is a serialized `CAIRNL1` field. A round-trip test
cannot catch a mirrored change (slice 2a: 19 of 19 mutations survived a green suite for exactly this
reason). Golden bytes captured from the **pre-newtype** build are the only thing that proves an
existing off-site export still restores after Task 5.

- [ ] **Step 1: Write the test with a deliberately wrong expectation**

```rust
//! Golden CBOR bytes for the `CAIRNL1` local-state bundle (issue #511).
//!
//! WHY THIS EXISTS: `LocalState::unwrap_secret` changes Rust type in this slice
//! (`Option<Vec<u8>>` -> `Option<Secret32>`), and it is a SERIALIZED field of the export a
//! restored clinic reads its custody out of. A round-trip test proves nothing about that: it
//! encodes and decodes through the same pair, so a mirrored change on both sides stays green
//! (DR slice 2a found 19 of 19 single-line mutations surviving for precisely this reason).
//! Only bytes frozen from the PREVIOUS build can fail.
//!
//! WHEN THIS FAILS: the on-disk export format has moved. Every `.localstate` file any operator
//! is holding off-site was written in the old shape. Do not re-freeze the constant to make it
//! green — that is the failure this test exists to prevent.

use cairn_node::localstate::{episode_dek_to_cbor, from_cbor, to_cbor, EpisodeDek, LocalState};

/// A 32-byte secret, derived at runtime — house rule 6(a). Never a byte-array literal in a
/// crypto context, even in a fixture.
fn secret_fixture(lineage: u8) -> Vec<u8> {
    (0u8..32)
        .map(|i| i.wrapping_mul(7).wrapping_add(lineage))
        .collect()
}

/// The bundle the golden bytes below encode. Every content slot is populated, because a pin
/// over an empty bundle would pin almost nothing: `unwrap_secret`'s encoding is the point.
fn populated_bundle() -> LocalState {
    let mut ls = LocalState::empty();
    ls.episode_deks = vec![episode_dek_to_cbor(&EpisodeDek {
        event_id: "00000000-0000-0000-0000-000000000001".to_string(),
        dek_wrapped: (0u8..104).map(|i| i.wrapping_mul(3)).collect(),
    })];
    ls.config = Some(b"config-blob".to_vec());
    ls.drafts = vec![b"a draft".to_vec()];
    ls.unwrap_secret = Some(secret_fixture(5));
    ls
}

/// The exact CBOR a pre-#511 build produces for [`populated_bundle`].
const POPULATED_BUNDLE_CBOR_HEX: &str = "00";

#[test]
fn the_populated_bundle_encodes_to_the_frozen_bytes() {
    assert_eq!(
        hex::encode(to_cbor(&populated_bundle())),
        POPULATED_BUNDLE_CBOR_HEX,
        "the CAIRNL1 bundle encoding moved — every off-site .localstate export was written \
         in the old shape"
    );
}

#[test]
fn the_frozen_bytes_decode_to_the_populated_bundle() {
    let bytes = hex::decode(POPULATED_BUNDLE_CBOR_HEX).expect("the pin is valid hex");
    let back = from_cbor(&bytes).expect("a bundle written by the previous build must still parse");
    assert_eq!(
        back,
        populated_bundle(),
        "a bundle written by the previous build decoded to something else"
    );
}
```

- [ ] **Step 2: Run it and read the real bytes out of the failure**

```bash
cd /Users/hherb/src/cairn-ehr
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test localstate_wire_pins
```

Expected: `the_populated_bundle_encodes_to_the_frozen_bytes` FAILS, printing the actual hex.

**This is the one place in this plan where the test is written to match the code, and that is
correct**: the current encoding *is* the format spec — it is what every existing export on every
operator's off-site disk was written in. Copy the actual hex from the failure output into
`POPULATED_BUNDLE_CBOR_HEX`.

- [ ] **Step 3: Re-run both tests**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test localstate_wire_pins
```

Expected: 2 passed.

- [ ] **Step 4: Prove the pin can actually fail (mutation check)**

Temporarily change `secret_fixture`'s `lineage` argument at its call site from `5` to `6`, re-run,
confirm BOTH tests fail, then change it back and confirm both pass again. A pin that cannot fail is
the `assert_eq!(SubjectKind::ALL.len(), 3)` trap — a guard defined over the thing it guards.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/tests/localstate_wire_pins.rs
git commit -m "$(cat <<'EOF'
test(#511): freeze the CAIRNL1 bundle bytes before the custody newtypes move them

LocalState::unwrap_secret changes Rust type in this slice and is a serialized
field of the export a restored clinic reads its custody out of. A round-trip
test cannot catch a mirrored change (slice 2a: 19/19 mutations survived a green
suite); only bytes frozen from the previous build can.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: The two types, used nowhere yet

**Files:**
- Create: `crates/cairn-event/src/keys.rs`
- Modify: `crates/cairn-event/src/lib.rs` (add `pub mod keys;`), `crates/cairn-event/Cargo.toml` (add `subtle`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces, for every later task:
  ```rust
  pub struct Secret32 { /* Zeroizing<[u8; 32]> */ }
  impl Secret32 {
      pub fn zeroed() -> Secret32;
      pub fn from_bytes(bytes: [u8; 32]) -> Secret32;
      pub fn from_slice(bytes: &[u8]) -> Option<Secret32>;
      pub fn as_bytes(&self) -> &[u8; 32];
      pub fn as_mut_bytes(&mut self) -> &mut [u8; 32];
  }
  // Clone, PartialEq (constant-time), Eq, Debug (redacting), Serialize, Deserialize

  #[derive(Clone, Copy, PartialEq, Eq, Debug)]
  pub struct PublicKey32 { /* [u8; 32] */ }
  impl PublicKey32 {
      pub fn from_bytes(bytes: [u8; 32]) -> PublicKey32;
      pub fn from_slice(bytes: &[u8]) -> Option<PublicKey32>;
      pub fn as_bytes(&self) -> &[u8; 32];
      pub fn to_bytes(self) -> [u8; 32];
  }
  // Serialize, Deserialize
  ```

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-event/src/keys.rs` containing ONLY the module doc and this test module (the
types come in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// House rule 6(a): derived at runtime, never a byte-array literal in a crypto context.
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
            "Debug leaked key material"
        );
    }

    /// A public key is published by design, so its Debug shows the real bytes — the type IS
    /// the argument that printing it is safe.
    #[test]
    fn a_public_key_debug_shows_its_bytes() {
        let p = PublicKey32::from_bytes(bytes_fixture(4));
        assert!(format!("{p:?}").contains(&format!("{}", bytes_fixture(4)[0])));
    }

    #[test]
    fn secrets_compare_by_value() {
        let a = Secret32::from_bytes(bytes_fixture(5));
        let b = Secret32::from_bytes(bytes_fixture(5));
        let c = Secret32::from_bytes(bytes_fixture(6));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// THE WIRE CONTRACT. `LocalState::unwrap_secret` was `Option<Vec<u8>>` and becomes
    /// `Option<Secret32>`; ciborium encodes a `Vec<u8>` as a CBOR ARRAY of unsigned ints
    /// (serde_bytes is not in play), so `Secret32` must encode identically or every existing
    /// off-site export stops restoring. Task 1's golden bytes are the end-to-end proof; this
    /// is the unit-level one, and it fails FIRST and more legibly.
    #[test]
    fn a_secret_encodes_exactly_as_the_vec_it_replaces() {
        let raw = bytes_fixture(7);
        let mut as_vec = Vec::new();
        ciborium::into_writer(&raw.to_vec(), &mut as_vec).unwrap();
        let mut as_secret = Vec::new();
        ciborium::into_writer(&Secret32::from_bytes(raw), &mut as_secret).unwrap();
        assert_eq!(
            as_secret, as_vec,
            "Secret32's CBOR must be byte-identical to the Vec<u8> it replaces in CAIRNL1"
        );
    }

    #[test]
    fn a_secret_decodes_from_the_vec_encoding_it_replaces() {
        let raw = bytes_fixture(8);
        let mut as_vec = Vec::new();
        ciborium::into_writer(&raw.to_vec(), &mut as_vec).unwrap();
        let back: Secret32 = ciborium::from_reader(&as_vec[..]).expect("must decode");
        assert_eq!(back.as_bytes(), &raw);
    }

    /// A wrong-length slot must be refused AT THE PARSE BOUNDARY, and must say why in words an
    /// operator can act on — the refusal `recovered_unwrap_secret` used to make one layer later.
    #[test]
    fn a_wrong_length_secret_is_refused_with_a_legible_cause() {
        let mut short = Vec::new();
        ciborium::into_writer(&vec![0u8; 31], &mut short).unwrap();
        let err = ciborium::from_reader::<Secret32, _>(&short[..])
            .expect_err("31 bytes cannot be a key");
        let text = format!("{err}");
        assert!(
            text.contains("32"),
            "the refusal must name the expected length; got: {text}"
        );
    }

    #[test]
    fn a_public_key_encodes_exactly_as_the_array_it_replaces() {
        let raw = bytes_fixture(9);
        let mut as_array = Vec::new();
        ciborium::into_writer(&raw, &mut as_array).unwrap();
        let mut as_public = Vec::new();
        ciborium::into_writer(&PublicKey32::from_bytes(raw), &mut as_public).unwrap();
        assert_eq!(as_public, as_array);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-event --lib keys
```

Expected: FAIL to compile — `cannot find type Secret32 in this scope`.

- [ ] **Step 3: Write the types**

Add `subtle = "2"` to `crates/cairn-event/Cargo.toml` `[dependencies]` with the licence comment the
file's convention requires, and `hex` / `ciborium` to `[dev-dependencies]` if not already present.
Add `pub mod keys;` to `crates/cairn-event/src/lib.rs`. Then, above the test module in `keys.rs`:

```rust
//! The custody plane's two key types — issue #511.
//!
//! WHY THIS EXISTS: every key here used to be a bare `[u8; 32]` — the X25519 secret half, its
//! public half, the node's Ed25519 signing seed and a per-event DEK, all the same type. So
//! installing a PUBLIC half as this node's SECRET custody key compiled, and `node_unwrap_key`
//! is a singleton whose registrar then refuses the real key forever: the #495 failure shape
//! (a restored solo clinic that can open none of its own record), reintroduced one layer up.
//!
//! WHAT THESE TWO TYPES DO AND DO NOT DO. Secret-vs-public is now a COMPILE error everywhere.
//! Secret-vs-secret is not: an unwrap secret, a signing seed and a DEK are all `Secret32`, so
//! `Secret32::from_bytes(sk.to_bytes())` still compiles. What changed is that it stopped being
//! an implicit coercion a reviewer cannot see and became a named, greppable line. That is the
//! same boundary `VerifiedKid` draws in `contributor.rs`, and it was a deliberate choice over
//! role-typed newtypes. `keystore::unwrap_secret_is_the_signing_seed` remains the only defence
//! against the signing-seed-as-unwrap-secret confusion.
//!
//! WHY A SEPARATE FILE: `seal.rs` is where these are USED and re-exported from (so
//! `cairn_event::seal::Secret32` resolves), but it is already past the project's 500-line
//! guideline. One definition, two paths, the second an ordinary re-export.

use serde::de::{Error as DeError, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// The length every key in this plane has. Named so the serde impls and the refusal message
/// cannot drift apart from each other.
const KEY_LEN: usize = 32;

/// 32 bytes of SECRET key material: an X25519 unwrap secret, an Ed25519 signing seed, a
/// per-event DEK, or a KEK. Wiped on drop; `Debug` redacts; equality is constant-time.
///
/// Deliberately NOT `Copy`: a `Copy` secret leaves unwiped duplicates by construction, which
/// is the opposite of what the `Zeroizing` inner buys.
#[derive(Clone)]
pub struct Secret32(Zeroizing<[u8; KEY_LEN]>);

impl Secret32 {
    /// An all-zero secret, to be filled in place via [`Self::as_mut_bytes`]. This keeps the
    /// "derive directly into the zeroizing buffer" idiom the crate already uses: the material
    /// never exists as a bare array on the stack that nothing can reach to wipe (#54).
    pub fn zeroed() -> Self {
        Secret32(Zeroizing::new([0u8; KEY_LEN]))
    }

    /// The ONE raw entry point. Deliberately named rather than a `From` impl: every place a
    /// loose 32 bytes becomes a secret should be findable with one grep, because that is where
    /// a signing seed could be misfiled as an unwrap secret (see the module doc).
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Secret32(Zeroizing::new(bytes))
    }

    /// Copy a slice into a pre-zeroed buffer, or refuse. `None` on any length but 32 — a
    /// wrong-length key is corruption or a version mismatch, never something to pad or truncate.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != KEY_LEN {
            return None;
        }
        let mut out = Self::zeroed();
        out.as_mut_bytes().copy_from_slice(bytes);
        Some(out)
    }

    /// The escape hatch, for feeding crypto primitives that take a raw array (AEAD keys,
    /// `StaticSecret::from`, HKDF input). Not for logging: use `{:?}`, which redacts.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Fill in place — see [`Self::zeroed`].
    pub fn as_mut_bytes(&mut self) -> &mut [u8; KEY_LEN] {
        &mut self.0
    }
}

/// Redacting rather than absent, and the difference matters. An absent `Debug` is a compile
/// error at each site that wants one, and the workaround a future author reaches for (an
/// accessor plus `hex::encode`) is strictly worse. A redacting one is a positive guarantee at
/// every present AND future site: it makes `#[derive(Debug)]` on a containing type safe, which
/// is how `localstate::LocalState` stopped needing a hand-written `Debug` whose only job was
/// redacting this one field.
impl std::fmt::Debug for Secret32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret32(<redacted>)")
    }
}

/// Constant-time, unlike the `*a != *b` array comparisons this type replaces. The threat is
/// thin here (these comparisons are local and an attacker does not choose the operands), but a
/// key-equality operator that short-circuits on the first differing byte is the kind of thing
/// that becomes load-bearing later, in a place nobody re-reads.
impl PartialEq for Secret32 {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&*other.0).into()
    }
}
impl Eq for Secret32 {}

/// WIRE-CRITICAL. `LocalState::unwrap_secret` was `Option<Vec<u8>>`, and ciborium encodes a
/// `Vec<u8>` as a CBOR ARRAY of unsigned ints (serde_bytes is not in play for that field). This
/// emits exactly that, so every `CAIRNL1` export written before #511 still restores and every
/// one written after is still readable by a build without these types.
/// Pinned end-to-end by `cairn-node/tests/localstate_wire_pins.rs`.
impl Serialize for Secret32 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter())
    }
}

/// Decodes into a PRE-SIZED ZEROIZING buffer — never into a `Vec<u8>` that would leave an
/// unwiped copy of the node's custody key in freed heap (#508's shape, narrowed here).
/// Refuses any length but 32 AT THE PARSE BOUNDARY, in words an operator can act on: this is
/// the refusal `localstate::recovered_unwrap_secret` used to make one layer later, moved to
/// where a corrupt bundle actually arrives — before anything is written or registered.
impl<'de> Deserialize<'de> for Secret32 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Secret32;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "exactly {KEY_LEN} bytes of key material")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Secret32, A::Error> {
                let mut out = Secret32::zeroed();
                let buf = out.as_mut_bytes();
                for (i, slot) in buf.iter_mut().enumerate() {
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
        d.deserialize_seq(V)
    }
}

/// 32 bytes of PUBLIC key material — an X25519 unwrap public half. Safe to log, store and
/// register: it is published by design (it sits in `node_unwrap_key` and travels in the
/// node-signed unwrap certificate), and it alone can never unwrap anything.
///
/// `Copy` is right here for the same reason it is wrong on [`Secret32`]: nothing needs wiping.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PublicKey32([u8; KEY_LEN]);

impl PublicKey32 {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        PublicKey32(bytes)
    }

    /// `None` on any length but 32 — the shape a public half takes coming off a database
    /// column or a wire field, where the length is not statically known.
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
```

- [ ] **Step 4: Run the tests**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-event --lib keys
```

Expected: 10 passed. If `a_public_key_encodes_exactly_as_the_array_it_replaces` fails, the derived
newtype `Serialize` is not transparent under ciborium — replace it with a hand-written
`serialize_tuple` of 32 elements matching `[u8; 32]`'s encoding, and say so in a comment.

- [ ] **Step 5: Confirm the rest of the workspace is untouched**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo check --workspace --all-targets
```

Expected: clean. Nothing uses the new types yet.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/keys.rs crates/cairn-event/src/lib.rs crates/cairn-event/Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
feat(#511): Secret32 and PublicKey32, the custody plane's two key types

Used nowhere yet. Secret-vs-public becomes a compile error in the next commit;
secret-vs-secret deliberately does not, and the module doc says so.

Serde is hand-written and wire-identical to the Vec<u8> it will replace in
CAIRNL1, decoding into a pre-sized zeroizing buffer and refusing a wrong-length
key at the parse boundary.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Migrate the whole custody plane

**Files:** every file in the File Structure table except the two docs and `localstate.rs`'s
producer-set work (Task 4).

**Interfaces:**
- Consumes: `Secret32`, `PublicKey32` from Task 2.
- Produces: the signature set in §4 of the spec.

**Why this is one task and not five.** A signature change in `cairn-event::seal` breaks every
downstream crate in the same edit; there is no ordering that leaves the tree compiling in between,
and house rule 6 (all tests pass before committing) forbids committing a broken tree. So this is one
atomic commit, gated by `cargo check` between crates rather than by intermediate commits.

- [ ] **Step 1: Re-export the types from `seal`**

At the top of `crates/cairn-event/src/seal.rs`, after the existing `use` block:

```rust
/// Re-exported so `cairn_event::seal::Secret32` — the path issue #511 names and the one every
/// call site in this plane uses — resolves. The definition lives in `crate::keys` because this
/// file is already past the 500-line guideline; see that module's header.
pub use crate::keys::{PublicKey32, Secret32};
```

- [ ] **Step 2: Migrate `cairn-event`, innermost first**

`seal.rs`, in this order (each is mechanical; the compiler names the next one):

```rust
pub fn seal_event_payload(..) -> Result<(serde_json::Value, Secret32), EventError>
//   let mut dek = Secret32::zeroed();  getrandom::fill(dek.as_mut_bytes())?;
pub fn unseal_event_payload(container: &serde_json::Value, dek: &Secret32, event_id: &str) -> ..
//   aead_key(dek.as_bytes())
pub fn generate_unwrap_secret() -> Result<Secret32, EventError>
pub fn derive_unwrap_secret(seed: &Secret32) -> Secret32
//   Hkdf::<Sha256>::new(None, seed.as_bytes()); hk.expand(.., out.as_mut_bytes())
pub fn unwrap_public(unwrap_secret: &Secret32) -> PublicKey32
//   PublicKey32::from_bytes(PublicKey::from(&StaticSecret::from(*unwrap_secret.as_bytes())).to_bytes())
fn wrap_kek(shared: &[u8], eph_pub: &PublicKey32, recipient_pub: &PublicKey32) -> Secret32
pub fn wrap_dek_for(dek: &Secret32, recipient_pub: &PublicKey32) -> Result<Vec<u8>, EventError>
pub fn unwrap_dek(wrapped: &[u8], unwrap_secret: &Secret32) -> Result<Secret32, EventError>
```

`aead_key` and `aead_nonce` keep their `&[u8; 32]` / `&[u8; 24]` arguments — they are the AEAD floor,
not a role in the custody plane. Reach them with `as_bytes()`.

In `lib.rs`:

```rust
pub fn sign_unwrap_key_cert(sk: &SigningKey, x25519_pub: &PublicKey32) -> Result<Vec<u8>, EventError>
pub fn verify_unwrap_key_cert(bytes: &[u8]) -> Result<(String, PublicKey32), EventError>
```

`verify_unwrap_key_cert`'s existing all-zero-key refusal stays exactly as it is, comparing
`x25519_pub.as_bytes() == &[0u8; 32]`.

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-event
```

Expected: green (its own `#[cfg(test)]` fixtures migrate with it — `seed_fixture`/`dek_fixture` return
`Secret32` via `Secret32::from_bytes(std::array::from_fn(..))`).

- [ ] **Step 3: Migrate `cairn-keystore`**

`seal.rs`: `seal(seed: &Secret32, ..)`, `unseal`/`unseal_op`/`unseal_rec` → `Option<Secret32>`,
`key_into_zeroizing` → `Secret32::from_slice`, `wrap_dek(dek: &Secret32, ..)`,
`try_unwrap(..) -> Option<Secret32>`. `aead_encrypt`/`aead_decrypt` keep `&[u8; 32]`.

`keystore.rs`: the eight signatures in spec §4. `adopt_derived_unwrap_secret` becomes the one site
that turns a signing seed into a `Secret32`, and its existing ⚠️ doc block gains one sentence saying
so:

```rust
pub fn adopt_derived_unwrap_secret(sk: &SigningKey) -> Secret32 {
    // The ONE production line in the tree that turns the Ed25519 signing seed into a
    // `Secret32`. `Secret32` does not distinguish an unwrap secret from a signing seed
    // (issue #511 §2), so this conversion is exactly the shape that, written anywhere else,
    // IS the #495 coupling — which is why it is named, commented, and pinned by
    // `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`.
    let seed = Secret32::from_bytes(sk.to_bytes());
    cairn_event::seal::derive_unwrap_secret(&seed)
}

pub fn unwrap_secret_is_the_signing_seed(unwrap: &Secret32, sk: &SigningKey) -> bool {
    *unwrap == Secret32::from_bytes(sk.to_bytes())
}
```

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-keystore
```

- [ ] **Step 4: Migrate `cairn-node`'s production sources**

`localstate.rs` — the type change only (the producer-set work is Task 4):

```rust
pub unwrap_secret: Option<Secret32>,     // was Option<Vec<u8>>
pub fn install(&self, secret: &Secret32) -> anyhow::Result<()>
fn verify_installed(path: &Path, reader: Option<&str>, expected: &Secret32, recipient: &str)
pub fn recovered_unwrap_secret(ls: &LocalState) -> Option<&Secret32>
pub fn secret_opens_the_carried_custody(ls: &LocalState, secret: &Secret32) -> anyhow::Result<()>
```

`recovered_unwrap_secret` loses its `Result`, because a wrong-length slot can no longer reach it —
`Secret32`'s `Deserialize` refuses it at `from_cbor`. **Rewrite its doc comment accordingly**: leaving
prose that argues for a refusal the function no longer performs is the #530 pattern (a comment
asserting a property its own code contradicts), which this repo has been bitten by twice in three
weeks. The new doc must say where the refusal moved to and why that placement is stronger.

`localstate_read.rs`: `unwrap_secret: Option<&Secret32>`, and the field becomes
`unwrap_secret: unwrap_secret.cloned()`.

`main.rs` (lines ~622, ~710, ~5316) and `medication/sealed_submit.rs` (~117): mechanical.
At `main.rs:5316` the registered public half comes off a DB column — use
`PublicKey32::from_slice(&bytes).ok_or_else(|| ..)` rather than a `try_into().expect(..)`.

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo check -p cairn-node --all-targets
```

- [ ] **Step 5: Migrate `cairn-sync`**

`unwrap_key.rs`: `FileOutcome::Loaded(Secret32)`, `Resolution::Use { secret: Secret32, .. }`,
`resolve(file, derived: Secret32, registered: Option<&PublicKey32>, path_display)`,
`registered_from_row(..) -> Result<Option<PublicKey32>, String>`,
`public_key_prefix(public: &PublicKey32) -> String` (`hex::encode(&public.as_bytes()[..8])`).

`public_key_prefix`'s doc block currently argues at length that CodeQL's `rust/cleartext-logging`
finding here is a false positive because `unwrap_public` is a one-way derivation the query cannot
model. **Add one sentence**: the argument is now also carried by the type — the parameter is
`PublicKey32`, which is public by construction. Do not delete the existing argument; the query still
fires, and #527's triage still owns dismissing it.

`main.rs`: `unwrap_secret: Option<&'a Secret32>` (~3641) — and **delete the three-line comment above
it** that explains why the field is `&[u8; 32]` rather than a plain slice, replacing it with one line
saying the fixed size is now carried by the type. `CustodyAdmission::Grant { requester_pub:
PublicKey32 }` (~5215), `decide_custody(kid, requester_pub: PublicKey32, lookup)` (~5270),
`fn ..(requester_pub: Option<&PublicKey32>, own_secret: Option<&Secret32>)` (~5466). `main.rs:558`
reads a hex signing seed from a file — that one stays `[u8; 32]`, it is an Ed25519 seed fed to
`SigningKey::from_bytes`, not custody-plane material.

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo check -p cairn-sync --all-targets
```

- [ ] **Step 6: Migrate the test fixtures**

`crates/cairn-node/tests/`: `common/mod.rs` (~865), `seal_submit.rs`, `seal_apply.rs`,
`dr_clinical_guarantee_gap.rs`, `restore_inherits_custody.rs`, `medication_authorship.rs`,
`authorship_binding.rs`; `crates/cairn-sync/tests/clinical_pull.rs`.

Two specific cases, not mechanical:

1. **`seal_apply.rs:917`** does `let mut wrong_dek: [u8; 32] = *dek;` then perturbs a byte. Becomes
   `let mut wrong_dek = dek.clone(); wrong_dek.as_mut_bytes()[0] ^= 1;`
2. **`restore_inherits_custody.rs:612`** puts the PUBLIC half in the secret slot to prove
   `secret_opens_the_carried_custody` catches it. That assignment no longer compiles — which is the
   whole point of this slice, and the test must **say so** rather than disappear. Convert it to a
   CBOR-level fixture (see Task 4 Step 3) and add a comment recording that the struct-level version
   of this mistake is now a compile error.

**Do NOT add a new `pub fn` to `crates/cairn-node/tests/common/mod.rs`** without also adding it to
`identity_scaffolding_shared.rs`'s hand-written expected-helper array, or
`derivation_finds_the_expected_helpers` fails.

- [ ] **Step 7: Migrate `extensions/cairn_pgx` (a separate cargo tree)**

`extensions/cairn_pgx/src/lib.rs` lines ~118, ~131–137, ~336–345:

```rust
let dek = cairn_event::seal::Secret32::from_slice(dek)?;                 // was try_into().ok()?
let unwrap_pub = cairn_event::seal::PublicKey32::from_slice(unwrap_pub)  // was try_into()
    .ok_or(..)?;
cairn_event::seal::wrap_dek_for(&dek, &unwrap_pub)
```

`attester_key` at ~144 is an Ed25519 verifying key, **not** custody-plane material — leave it
`[u8; 32]`.

Then update the guard fixture in `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs` (lines
~361, ~383, ~399, ~404, ~437): these are synthetic source strings feeding `is_the_declaration_line`
and the `#[cfg(test)]` tail matcher. They must be rewritten to the real new signature
(`pub fn derive_unwrap_secret(seed: &Secret32) -> Secret32 {`) so the guard's fixtures do not assert a
signature the tree no longer has. The guard's *logic* is unchanged — it matches on the
`fn derive_unwrap_secret(` prefix, which is unaffected.

- [ ] **Step 8: Refresh all three lockfiles and run the gates**

```bash
cd /Users/hherb/src/cairn-ehr && cargo update -p cairn-event --workspace 2>/dev/null; cargo check --workspace --all-targets
cd /Users/hherb/src/cairn-ehr/extensions/cairn_pgx && cargo check --all-targets --locked || cargo check --all-targets
cd /Users/hherb/src/cairn-ehr/cairn-gui && cargo check --all-targets --locked || cargo check --all-targets
```

If either `--locked` run fails on a stale lockfile, re-run it without `--locked` to regenerate, then
confirm `--locked` passes. **No root-workspace gate sees these two lockfiles** — this step is the
only thing standing between a green local run and a red CI.

- [ ] **Step 9: Prove the defect is actually fixed**

Add to `crates/cairn-event/src/keys.rs`'s test module a **compile-fail record**, since Rust has no
`#[should_not_compile]` without a `trybuild` dependency this project does not carry:

```rust
/// THE DEFECT THIS SLICE EXISTS TO FIX, recorded as prose beside a positive control because a
/// compile error cannot be asserted without a `trybuild` dependency this tree does not carry.
///
/// These three lines are what #511 reported, and NONE of them compiles any more:
///
/// ```compile_fail
/// # use cairn_event::keys::{PublicKey32, Secret32};
/// # fn install(_: &Secret32) {}
/// # let secret = Secret32::from_bytes([0u8; 32]);
/// let public: PublicKey32 = cairn_event::seal::unwrap_public(&secret);
/// install(&public);
/// ```
///
/// The positive control below proves the doctest harness is actually running these — a
/// `compile_fail` block that fails for the WRONG reason (a typo, a missing import) passes
/// silently and guards nothing.
#[test]
fn the_public_for_secret_confusion_is_recorded() {
    // Positive control: the correct call DOES compile and round-trips.
    let secret = Secret32::from_bytes(bytes_fixture(11));
    let public = crate::seal::unwrap_public(&secret);
    assert_ne!(public.as_bytes(), secret.as_bytes());
}
```

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-event --doc
```

Expected: the `compile_fail` doctest passes (i.e. the code inside it does not compile).

- [ ] **Step 10: Run the DB-gated suites**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace 2>&1 | tee /tmp/claude-501/-Users-hherb-src-cairn-ehr/gate.log; echo "exit=$?"
```

Never pipe to `tail` — it masks cargo's exit code. Expect ~2 hours for a cross-cutting change (~134
test binaries relink; the stall between binaries is macOS's one-time-per-binary Gatekeeper
assessment, not cargo). Start it in the background and do Task 5's docs pass while it runs.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(#511): the custody plane takes Secret32 and PublicKey32, not bare [u8; 32]

Installing a PUBLIC half as this node's SECRET custody key is now a compile
error, across cairn-event, cairn-keystore, cairn-node, cairn-sync and the
cairn_pgx tree. node_unwrap_key is a singleton, so that mistake forecloses the
real key permanently — the #495 shape one layer up, and the read-after-write
checks could never catch it (they prove the file holds the bytes we wrote,
never a key that opens anything).

Secret-vs-secret is deliberately NOT separated: an unwrap secret, a signing
seed and a DEK are all Secret32. What changed is that confusing them stopped
being an implicit coercion and became a named, greppable line —
keystore::adopt_derived_unwrap_secret is the one production site that makes it.

CAIRNL1 bytes are unchanged, pinned by localstate_wire_pins.rs. A wrong-length
secret is now refused at the parse boundary instead of at install.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Close `LocalState`'s producer set and make wiping structural

**Files:**
- Modify: `crates/cairn-node/src/localstate.rs`, `crates/cairn-node/src/localstate_read.rs`,
  `crates/cairn-node/tests/restore_inherits_custody.rs`,
  `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs`

**Interfaces:**
- Consumes: `Secret32` from Task 2; the migrated `LocalState` from Task 3.
- Produces:
  ```rust
  impl LocalState {
      pub fn empty() -> LocalState;
      pub fn from_custody(episode_deks: Vec<Vec<u8>>, unwrap_secret: Option<Secret32>) -> LocalState;
      pub fn version(&self) -> u8;
      pub fn node_default_deks(&self) -> &[Vec<u8>];
      pub fn episode_deks(&self) -> &[Vec<u8>];
      pub fn config(&self) -> Option<&[u8]>;
      pub fn drafts(&self) -> &[Vec<u8>];
      pub fn unwrap_secret(&self) -> Option<&Secret32>;
      pub fn set_unwrap_secret(&mut self, secret: Option<Secret32>);
      pub fn take_unwrap_secret(&mut self) -> Option<Secret32>;
      pub fn set_drafts(&mut self, drafts: Vec<Vec<u8>>);
      pub fn is_empty(&self) -> bool;
  }
  ```

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/localstate.rs`:

```rust
/// #511 rides-along 1. `LocalState`'s own doc says a third producer skipping the
/// `erasure_shred_log` filter "is how an erased body's key would travel" — and until now
/// nothing prevented one, because every field was `pub`. There is deliberately no
/// `set_episode_deks`: that is the slot the filter guards, and the only way to fill it is
/// `from_custody`, which `read_local_state` (the filtering producer) calls.
#[test]
fn from_custody_is_the_only_way_to_fill_the_custody_slot() {
    let secret = cairn_event::seal::Secret32::from_bytes(std::array::from_fn(|i| {
        (i as u8).wrapping_mul(7).wrapping_add(1)
    }));
    let ls = cairn_node::localstate::LocalState::from_custody(
        vec![b"a wrapped row".to_vec()],
        Some(secret.clone()),
    );
    assert_eq!(ls.episode_deks().len(), 1);
    assert_eq!(ls.unwrap_secret(), Some(&secret));
    assert!(!ls.is_empty());
}

/// #511 rides-along 2. The hand-written `Debug` that existed only to redact `unwrap_secret`
/// is gone; redaction is now `Secret32`'s, so it covers every future secret-bearing slot
/// automatically instead of being re-earned per field.
#[test]
fn debug_still_redacts_the_secret_without_a_hand_written_impl() {
    let raw: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(11).wrapping_add(3));
    let mut ls = cairn_node::localstate::LocalState::empty();
    ls.set_unwrap_secret(Some(cairn_event::seal::Secret32::from_bytes(raw)));
    let shown = format!("{ls:?}");
    assert!(shown.contains("<redacted>"), "got: {shown}");
    assert!(
        !shown.contains(&hex::encode(raw)[..8]),
        "the bundle's Debug leaked the node's custody key: {shown}"
    );
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test localstate
```

Expected: FAIL to compile — no `from_custody`, no `episode_deks()`.

- [ ] **Step 3: Implement**

In `localstate.rs`: every field on `LocalState`, `LskWraps` and `SealedLocalState` becomes
`pub(crate)`; add the accessors and the two narrow mutators above; add
`LskWraps::new(..)` / `SealedLocalState::new(wraps, payload_nonce, payload_ct)`; replace the
hand-written `Debug` with `#[derive(Debug)]`; widen `Drop`:

```rust
/// Wipes the slots `Secret32` does not cover.
///
/// `unwrap_secret` is a `Secret32` now, so its own `Drop` wipes it and this impl no longer
/// names it — that is the point of the type. `node_default_deks` is the RESERVED, untyped slot
/// (`Vec<Vec<u8>>`, no producer exists yet), so it is not covered structurally and is wiped
/// here by name. This is a no-op today and correct the day the slot is filled; #511's warning
/// was precisely that a `Drop` naming one field goes stale silently when a second is added.
///
/// LIMITS, unchanged: serde makes its own copies during encode and decode, and a `Vec` that
/// reallocated while being built leaves its earlier buffer behind. Wiping is a real reduction,
/// not an erasure guarantee (#508).
impl Drop for LocalState {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for k in self.node_default_deks.iter_mut() {
            k.zeroize();
        }
    }
}
```

In `localstate_read.rs`, replace the struct literal with
`Ok(LocalState::from_custody(episode_deks, unwrap_secret.cloned()))`.

Then fix the test call sites: `.episode_deks` → `.episode_deks()`, `.unwrap_secret` →
`.unwrap_secret()`, `bundle.unwrap_secret = Some(x)` → `bundle.set_unwrap_secret(Some(x))`,
`bundle.drafts = v` → `bundle.set_drafts(v)`, `bundle.unwrap_secret.take()` →
`bundle.take_unwrap_secret()`.

The two tests that build a **truncated** secret (`restore_inherits_custody.rs:157` and ~411) can no
longer do it through the struct. Rewrite them against CBOR, which is how a corrupt bundle actually
arrives — declare a local mirror struct in the test file:

```rust
/// A `LocalState` in the shape a FOREIGN or CORRUPT bundle can take: the secret slot is a raw
/// `Vec<u8>` of any length. This is what a truncated export looks like on disk, and since #511
/// it is the only way to build one — `LocalState::unwrap_secret` is a `Secret32`, so a
/// wrong-length secret is unrepresentable in the real type. The refusal therefore moved from
/// `recovered_unwrap_secret` (at install) to `from_cbor` (at parse), which is earlier and
/// strictly better: nothing is written before it fires.
#[derive(serde::Serialize)]
struct ForeignBundle {
    version: u8,
    node_default_deks: Vec<Vec<u8>>,
    episode_deks: Vec<Vec<u8>>,
    config: Option<Vec<u8>>,
    drafts: Vec<Vec<u8>>,
    unwrap_secret: Option<Vec<u8>>,
}
```

and assert `from_cbor` refuses with a message naming `32`.

- [ ] **Step 4: Run the tests**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test localstate --test localstate_wire_pins --test restore_inherits_custody --test dr_clinical_guarantee_gap
```

Expected: all green — **including Task 1's golden bytes**, which is the proof the format did not move.

- [ ] **Step 5: Confirm the producer-count guard still sees exactly two**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test dr_clinical_guarantee_gap
```

`empty()` and `from_custody()` are the two `LocalState {` literals; `read_local_state` no longer
contains one. If the guard names specific files rather than counting across the tree, update its
expectation and say in a comment why the site moved.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(#511): close LocalState's producer set and make redaction structural

Its own doc said a third producer skipping the erasure_shred_log filter "is how
an erased body's key would travel" — and every field was pub. Fields are
pub(crate) now, with from_custody() the only way to fill the custody slot;
there is deliberately no set_episode_deks.

The hand-written Debug is gone: redaction is Secret32's, so it covers every
future secret-bearing slot instead of being re-earned per field. Drop is kept
and widened to the reserved untyped node_default_deks slot rather than deleted —
a no-op today, correct the day that slot is filled.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Docs, gates, PR

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Update `docs/ROADMAP.md`** — add the slice entry under Phase 5, naming what it closes
  (#511) and what it explicitly does not (#500, #508). Keep the file under 500 lines by condensing an
  older slice; **never drop an open issue number while condensing** (the PR #271 finding).

- [ ] **Step 2: Update `docs/HANDOVER.md`** — new ⇒ NEXT (slice 2c), a "Recent sessions" entry, the
  session-date line, and **delete the ⇒ #511 warning block**, which asserts `grep -rn
  'Secret32\|PublicKey32' crates/` returns nothing. Leaving it is the exact stale-deferral failure
  this repo has now been bitten by three times. Keep under 500 lines.

- [ ] **Step 3: Run the full gate and the doc gate**

```bash
cd /Users/hherb/src/cairn-ehr
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cd extensions/cairn_pgx && cargo clippy --all-targets --locked -- -D warnings
cd ../../cairn-gui && cargo clippy --all-targets --locked -- -D warnings
```

CI's `cargo doc` runs with `RUSTDOCFLAGS=-D warnings`, and a `[private_helper]` intra-doc link to a
non-public item fails TWO jobs — `run-db-gated-tests.sh` does not run `cargo doc`, so this step is the
only local coverage of it.

- [ ] **Step 4: Read the open CodeQL alerts before pushing**

```bash
scripts/codeql-alerts.sh
```

Read-only (`gh api` is deny-listed repo-wide and must stay so). The new `PublicKey32` parameter names
in `cairn-sync` sit exactly where the 12 `rust/cleartext-logging` alerts already fire; confirm the
count has not grown, and do not assume a new finding is the familiar false positive — alert #24 was
assumed to be one for a week and was a real defect.

- [ ] **Step 5: Push and open the PR**

```bash
git push -u origin feat/511-custody-newtypes-secret32-publickey32
gh pr create --base main --title "#511: the custody plane takes Secret32 and PublicKey32, not bare [u8; 32]" --body "$(cat <<'EOF'
Closes #511.

... (see the design doc for the full argument; the PR body states what compiles
now and what deliberately still does, the CAIRNL1 wire pin, and the two
rides-along items) ...

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review notes

- **Spec coverage.** §3 (type contract) → Task 2. §4 (migrated surface) → Task 3. §5 (wire risk) →
  Tasks 1, 2 Step 3, 4 Step 4. §6 rides-along 1 and 2 → Task 4; rides-along 3 (`LskWraps` /
  `SealedLocalState` constructors) → Task 4 Step 3. §7 (what this does not do) → recorded in the
  commit message and the module doc, not in code. §8 (gates) → Task 5.
- **Known sequencing hazard.** Task 3 is one large atomic commit because no ordering leaves the tree
  compiling in between. Its `cargo check` gates between crates are the substitute for intermediate
  commits, and Step 10's full `cargo test` is the real gate.
- **The residual this plan does not close.** `Secret32::from_bytes(sk.to_bytes())` still compiles.
  That is the accepted line (spec §2), and the mitigation is that
  `keystore::adopt_derived_unwrap_secret` is the single production site making that conversion, is
  commented as such, and is already pinned by `unwrap_secret_is_not_derived.rs`.
