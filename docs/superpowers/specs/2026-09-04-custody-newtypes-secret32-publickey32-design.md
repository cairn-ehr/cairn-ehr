# Custody newtypes — `Secret32` / `PublicKey32` (design)

**Issue:** [#511](https://github.com/cairn-ehr/cairn-ehr/issues/511)
**Sequenced:** after DR slice 2b, **before slice 2c** — 2c/2d are where key material starts moving
again (the medium carrying DEKs, `CAIRNL1` carrying the unwrap secret), so the newtypes must exist
before that code is written rather than be retrofitted onto it.
**Closes:** #511. **Does not touch #500** — the medium still carries no clinical event.
**No ADR, no spec bump, no migration, no DB change.**

---

## 1. The defect

Every key in the custody plane is the same type — a bare `[u8; 32]`:

| distinction | expressed in the type system today? |
|---|---|
| X25519 secret half vs its public half | ❌ both `[u8; 32]` |
| X25519 unwrap secret vs Ed25519 signing seed | ❌ both `[u8; 32]` |
| DEK vs unwrap secret | ❌ both `&[u8; 32]` |

So all three of these compile:

```rust
let public = cairn_event::seal::unwrap_public(&secret);
destination.install(&public)?;                                  // 1. PUBLIC half as the secret
keystore::write_unwrap_sealed(&path, &sk.to_bytes(), op, &code)?; // 2. SIGNING SEED as the secret
localstate::read_local_state(db, Some(&public)).await?;          // 3. PUBLIC half exported
```

`node_unwrap_key` is a **singleton**: registering the wrong public half forecloses the real key
permanently. Under the §9 blast-radius rule a defect here silently orphans the entire clinical record
of a restored solo clinic — the #495 failure shape, reintroduced one layer up.

The existing defences are all runtime and all partial:

- `CustodyKeyDestination::install`'s and `keystore::generate_unwrap_sealed`'s read-after-write checks
  prove the file holds **the bytes we wrote**, never **a key that opens anything**.
- `localstate::secret_opens_the_carried_custody` (PR #510) trial-unwraps one carried DEK — it closes
  the *restore* path only, and only when the bundle carries custody.
- `keystore::unwrap_secret_is_the_signing_seed` is wired on the **read** path only, never the write.

## 2. What this design does — and what it deliberately does not

Two types in `cairn-event`, per #511 as written:

```rust
/// 32 bytes of SECRET key material. Wiped on drop; Debug redacts; no derived Serialize.
pub struct Secret32(Zeroizing<[u8; 32]>);

/// 32 bytes of PUBLIC key material. Safe to log, store, register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PublicKey32([u8; 32]);
```

**Distinction 1 becomes a compile error everywhere.** That is the catastrophic-and-silent class.

**Distinctions 2 and 3 do NOT become compile errors, and this design says so plainly.** Unwrap
secret, signing seed and DEK are all `Secret32`. The three lines above stop compiling *as written* —
`sk.to_bytes()` is `[u8; 32]`, not `Secret32` — but a deliberate `Secret32::from_bytes(sk.to_bytes())`
still compiles. What changes is that the confusion stops being an implicit coercion a reviewer cannot
see and becomes a **named, greppable line**. That is the same line `VerifiedKid`
(`crates/cairn-event/src/contributor.rs`) draws — one wrapper, not one per role — and it is the line
the maintainer chose over role-typed newtypes (`UnwrapSecret`/`SigningSeed`/`Dek`) on 2026-09-04.

Residual, stated rather than implied: **`keystore::unwrap_secret_is_the_signing_seed` remains the only
defence against distinction 2**, and it is still wired only where both keys are genuinely this node's
(`establish-unwrap-key`). This design does not widen it — see §7.

## 3. Type contract

### `Secret32`

| property | choice | why |
|---|---|---|
| inner | `Zeroizing<[u8; 32]>` | wiped on drop, structurally — not per-field-by-name |
| `Debug` | **hand-written, redacting** (`Secret32(<redacted>)`) | see below |
| `Serialize`/`Deserialize` | **hand-written**, wire-identical to `Vec<u8>` | §5 |
| `PartialEq`/`Eq` | constant-time via `subtle::ConstantTimeEq` | already a transitive dep (2.6.1, BSD-3-Clause, AGPL-compatible); today's `*a != *b` comparisons are variable-time |
| `Clone` | yes | `Zeroizing` clones; every copy still wipes |
| `Copy` | **no** | a `Copy` secret leaves unwiped duplicates by construction |

**Redacting `Debug`, not absent `Debug` — a deliberate deviation from #511's "never Debug".**
Absence of `Debug` is a compile error at each site that wants one, and the workaround a future author
reaches for (an accessor plus `hex::encode`) is strictly worse than a redaction. A redacting `Debug`
is a *positive* guarantee at every present and future site: it makes `#[derive(Debug)]` on a
containing type safe. It is also what lets `localstate::LocalState` drop its hand-written `Debug`
(which exists solely to redact this one field) and derive it instead — redaction becomes structural,
the same win as wiping.

Constructors — the complete, greppable set:

```rust
Secret32::zeroed() -> Secret32                       // then fill via as_mut_bytes()
Secret32::from_bytes(bytes: [u8; 32]) -> Secret32    // the ONE raw entry point
Secret32::from_slice(bytes: &[u8]) -> Option<Secret32> // copies into a pre-zeroed buffer; None on len != 32
```

Accessors:

```rust
fn as_bytes(&self) -> &[u8; 32]          // the escape hatch, for crypto primitives only
fn as_mut_bytes(&mut self) -> &mut [u8; 32] // fill-in-place, keeps the "derive directly into the
                                            // zeroizing buffer" idiom this tree already uses
```

### `PublicKey32`

```rust
PublicKey32::from_bytes([u8; 32]) -> PublicKey32
fn as_bytes(&self) -> &[u8; 32]
fn to_bytes(self) -> [u8; 32]
```

`Debug`/`Display` may print it in full: a node's unwrap public key is published by design (registered
in `node_unwrap_key`, carried in the node-signed unwrap cert). CodeQL's `rust/cleartext-logging`
already flags the `cairn-sync` sites that print a prefix of it; `PublicKey32` makes the argument
*"this is the public half"* a type fact rather than a comment, which is the thing #529 asks for.

### Where the types live

`crates/cairn-event/src/keys.rs` — a new focused file, because `seal.rs` is already 640 lines and
house rule 4 aims under 500. `seal.rs` re-exports them (`pub use crate::keys::{PublicKey32, Secret32};`)
so `cairn_event::seal::Secret32` — the path #511 names — resolves. One definition, two paths, the
second an ordinary Rust re-export.

## 4. Migrated surface

The whole custody plane, four workspace crates plus the separately-`exclude`d `extensions/cairn_pgx`
tree. Signatures after the change:

```rust
// cairn-event::seal
seal_event_payload(..) -> Result<(Value, Secret32), EventError>
unseal_event_payload(container, dek: &Secret32, event_id) -> ..
generate_unwrap_secret() -> Result<Secret32, EventError>
derive_unwrap_secret(seed: &Secret32) -> Secret32
unwrap_public(unwrap_secret: &Secret32) -> PublicKey32
wrap_dek_for(dek: &Secret32, recipient_pub: &PublicKey32) -> Result<Vec<u8>, EventError>
unwrap_dek(wrapped: &[u8], unwrap_secret: &Secret32) -> Result<Secret32, EventError>

// cairn-event (lib.rs)
sign_unwrap_key_cert(sk, x25519_pub: &PublicKey32) -> Result<Vec<u8>, EventError>
verify_unwrap_key_cert(bytes) -> Result<(String, PublicKey32), EventError>

// cairn-keystore
seal::seal(seed: &Secret32, ..) -> Result<SealedKey, SealError>
seal::unseal / unseal_op / unseal_rec(..) -> Option<Secret32>
keystore::write_unwrap_sealed(path, secret: &Secret32, ..) -> Result<(), KeystoreError>
keystore::write_unwrap_plaintext(path, secret: &Secret32) -> Result<(), KeystoreError>
keystore::generate_unwrap_sealed(..) -> Result<PublicKey32, KeystoreError>
keystore::generate_unwrap_plaintext(path) -> Result<PublicKey32, KeystoreError>
keystore::load_unwrap_secret(path, secret) -> Result<Secret32, KeystoreError>
keystore::adopt_derived_unwrap_secret(sk) -> Secret32
keystore::unwrap_secret_is_the_signing_seed(unwrap: &Secret32, sk) -> bool

// cairn-node
CustodyKeyDestination::install(&self, secret: &Secret32) -> anyhow::Result<()>
localstate::recovered_unwrap_secret(ls) -> Option<&Secret32>          // note: no longer fallible, §5
localstate::secret_opens_the_carried_custody(ls, secret: &Secret32) -> anyhow::Result<()>
localstate_read::read_local_state(db, unwrap_secret: Option<&Secret32>) -> anyhow::Result<LocalState>

// cairn-sync
unwrap_key::FileOutcome::Loaded(Secret32)
unwrap_key::Resolution::Use { secret: Secret32, warning: Option<String> }
unwrap_key::resolve(file, derived: Secret32, registered: Option<&PublicKey32>, path) -> Resolution
```

**Crypto primitives stay on `&[u8; 32]`** — `seal::aead_key`, `keystore::seal::aead_encrypt` /
`aead_decrypt`, `wrap_kek`'s output feeding them. The *plane* is what gets typed; the AEAD floor
underneath it is reached through `as_bytes()`. Typing the primitives too would add churn without
adding a distinction: an AEAD key is not a role in the custody plane.

## 5. The wire risk, and how it is pinned

`LocalState::unwrap_secret` is `Option<Vec<u8>>` today and becomes `Option<Secret32>`. **This is a
serialized field of the `CAIRNL1` export**, so the encoding must not move: ciborium encodes `Vec<u8>`
as a CBOR **array of 32 unsigned ints** (not a byte string — serde_bytes is not in play here), and
`Secret32`'s hand-written `Serialize` must emit exactly that.

A round-trip test cannot catch a mirrored change — slice 2a's lesson, where 19 of 19 single-line
mutations survived a green suite because every test round-tripped through the same encoder/decoder
pair. So the pin is **golden bytes**, and it is captured **before** the type changes:
`crates/cairn-node/tests/localstate_wire_pins.rs` freezes the exact CBOR of a fully-populated
`LocalState` produced by the pre-newtype build, and decodes those same frozen bytes back. Both halves
must stay green through the migration. That is what proves an existing off-site export still restores.

**Consequence to state, not to discover later.** With `Option<Secret32>` and a `Deserialize` that
accepts exactly 32 elements, a malformed-length secret slot becomes **unrepresentable**, so
`recovered_unwrap_secret`'s length refusal moves from the *install* boundary to the *parse* boundary
(`from_cbor`). That is strictly better placement — a corrupt bundle is refused before anything is
written, which is what its own doc already argues for — but the operator-facing sentence must travel
with it into the deserializer's error, or a real corruption degrades from a named cause to
`Decode("invalid type")`. `recovered_unwrap_secret` accordingly becomes infallible
(`-> Option<&Secret32>`), and the two tests that build a truncated secret by mutating the struct move
to building CBOR bytes — which is how a corrupt bundle actually arrives.

## 6. Rides along (all three named in #511)

1. **`LocalState`'s producer set becomes closed.** Fields go `pub(crate)`; producers are `empty()`
   and a new `from_custody(episode_deks, unwrap_secret)` that `read_local_state` calls. Its own doc
   already says a third producer skipping the `erasure_shred_log` filter *"is how an erased body's
   key would travel"* — and nothing prevented one. Read accessors replace the public fields.
   Two narrow mutators (`set_unwrap_secret`, `set_drafts`) exist for the slots tests legitimately
   need to vary; there is deliberately **no** `set_episode_deks`, because that is the slot the filter
   guards. `dr_clinical_guarantee_gap.rs`'s producer-count guard must still see exactly two
   `LocalState {` literals.
2. **Wiping becomes structural.** `Secret32`'s own `Drop` covers `unwrap_secret`, so `LocalState`'s
   hand-written `Drop` no longer needs to name it — but the reserved, untyped `node_default_deks`
   slot is not `Secret32` and is not covered. `Drop` is therefore **kept and widened** to wipe that
   slot too (a no-op today, correct the day the slot is filled) rather than deleted. #511's deeper
   point — *"every future secret-bearing slot re-opens it"* — is answered for every typed slot and
   stated honestly for the untyped one. This narrows #508's blast radius; it does not close #508
   (serde still makes intermediate copies).
3. **`LskWraps` / `SealedLocalState` get constructors** and `pub(crate)` fields, so a
   `SealedLocalState` pairing node A's wraps with node B's payload stops being representable.

## 7. What this does NOT do

- **Does not close #500.** No clinical event travels on any medium as a result of this slice.
- **Does not wire `unwrap_secret_is_the_signing_seed` onto the write path.** `write_unwrap_sealed`'s
  two callers are the adoption migration (whose secret is an HKDF *derivation* of the seed and so can
  never equal it) and `restore` (where the live signing key is a different key entirely and the
  comparison is meaningless — the function's own doc says so). Adding a check whose justification does
  not hold at either call site is the *"a wrong safety argument is worse than none"* trap.
- **Does not make the DEK a distinct type from the unwrap secret.** §2.
- **Adds no dependency to `cairn-event` beyond promoting `subtle`** from transitive to direct
  (2.6.1, BSD-3-Clause, AGPL-3.0-compatible, already in `Cargo.lock`, already audited by cargo-deny
  through the dalek crates).

## 8. Gates

`cargo test` on the root workspace, `--locked` clippy on **all three cargo trees**
(root, `extensions/cairn_pgx`, `cairn-gui`) — a new/changed dependency edge in a root crate makes the
two `exclude`d trees' lockfiles stale and **no root-workspace gate sees it**. `cargo doc` with
`RUSTDOCFLAGS=-D warnings`. `CAIRN_ALLOW_DB_SKIP=1` for a DB-free run (#450).
