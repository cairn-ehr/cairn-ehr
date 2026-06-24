# Design — At-rest keystore seal + recovery escrow (ADR-0026 slice A)

**Date:** 2026-06-24
**Status:** Approved (brainstorming) → ready for implementation plan
**ADR:** [ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) — node durability & disaster recovery
**Scope:** `crates/cairn-node` only. No spec/ADR change. No DB/schema change.

## Problem

`cairn-node` writes its Ed25519 signing key to disk as **plaintext, mode 0600**
([`keystore.rs`](../../../crates/cairn-node/src/keystore.rs)). The `passphrase` argument the
keystore API accepts is silently ignored — there is no KDF and no encryption. The node advertises
this honestly: `status` reports `key_at_rest PLAINTEXT` and `dr_escrow STUBBED (key loss = node loss)`.

ADR-0026 names exactly one **day-one, can't-retrofit** requirement: the recovery-secret escrow and a
sealed local-state export must exist at provisioning. This slice delivers the foundational, lowest-
blast-radius part of that requirement: **seal the signing key at rest, and mint an off-node recovery
secret at provisioning.** It is the prerequisite the keystore code itself flags, and it closes the two
honest gaps the node currently advertises.

This slice is deliberately **not** the whole ADR. Explicit non-goals are listed at the end.

## Decisions (settled in brainstorming)

1. **Dual-recipient seal.** A random 32-byte Data Encryption Key (DEK) seals the signing seed. The DEK
   is then *wrapped twice*: once under a key derived from an **operational passphrase** (for unattended
   `run`/`serve`), once under a key derived from a **one-time recovery code** (paper escrow, displayed
   once at `init`). Daily operations never need the paper code; the paper code survives a forgotten
   passphrase and is the off-node escrow ADR-0026 point 5 requires.
2. **KDF = Argon2id; AEAD = XChaCha20-Poly1305** (RustCrypto `argon2` + `chacha20poly1305`, both dual
   MIT/Apache-2.0 → AGPL-3.0-compatible; pure-Rust, reviewer-legible per §9). XChaCha20's 192-bit nonce
   lets us use a fresh random nonce per seal with no reuse concern.
3. **Refuse silent plaintext.** `init`/`seal-key` require an operational passphrase (flag / env /
   prompt). Plaintext is reachable only via an explicit `--insecure-plaintext` escape hatch (throwaway
   test nodes).
4. **Pure primitives in a new `seal.rs`** module inside `cairn-node`. Promote to a shared crate later
   only if the point-3 sealed-state export reuses it.
5. **No DB/event change.** Escrow presence is a *local* fact derived from the sealed file containing a
   recovery wrap. `status` reads the file. (An audited escrow-generation event is a deferred non-goal.)

## Architecture

### New module: `crates/cairn-node/src/seal.rs` (pure, safety-critical)

The small surface ADR-0026 point 3 calls out — "the only component that touches private key material."
All functions are pure given their inputs (entropy is passed in or drawn from `getrandom` at the
boundary), so they are unit-testable without a DB or filesystem.

```text
struct SealedKey {                 // <-> versioned CBOR bytes on disk
    version: u8,                   // format version (1)
    argon: ArgonParams,           // m_cost, t_cost, p_cost (pinned, in-file for forward compat)
    salt_op:  [u8; 16],
    salt_rec: [u8; 16],
    seed_nonce: [u8; 24],          // XChaCha20 nonce for DEK -> seed
    seed_ct:    Vec<u8>,           // AEAD(seed) under DEK  (+ tag)
    wrap_op:  Wrap,                // { nonce[24], ct: AEAD(DEK) under KEK_op }
    wrap_rec: Wrap,                // { nonce[24], ct: AEAD(DEK) under KEK_rec }
}

fn seal(seed: &[u8;32], op_pass: &str, recovery_code: &str) -> Result<SealedKey>
fn unseal(s: &SealedKey, secret: &str) -> Option<[u8;32]>   // tries op-wrap then rec-wrap
fn to_cbor(&SealedKey) -> Vec<u8>     // magic "CAIRNK1\n" prefix + ciborium
fn from_cbor(&[u8]) -> Result<SealedKey>

fn generate_recovery_code() -> String                 // 160-bit random, Crockford base32, grouped
fn base32_encode(&[u8]) -> String / base32_decode(&str) -> Option<Vec<u8>>   // pure, tiny
```

- `unseal` returns `None` on a wrong secret or any tampered field (the AEAD tag fails) — there is no
  separate integrity check; the seal is self-verifying, mirroring the verify-on-apply posture of the
  rest of Cairn.
- `to_cbor` prepends a magic header so `keystore::load` can distinguish a sealed bundle from a raw
  32-byte plaintext seed by inspection.

### Changes: `keystore.rs`

- `generate_and_seal(path, op_pass, recovery_code) -> (SigningKey, kid)` — generate keypair, `seal`,
  write the CBOR bundle (0600). Caller already holds the recovery code to display.
- `load(path, secret: Option<&str>) -> SigningKey` — **auto-detect**: magic header → `from_cbor` +
  `unseal(secret)` (legible error if `secret` absent or wrong); exactly 32 bytes → plaintext load.
- `seal_existing(path, op_pass, recovery_code)` — read a plaintext seed, seal it, rewrite (the
  `seal-key` migration).
- `key_at_rest_state(path) -> KeyAtRest { Sealed { dual_recipient: bool } | Plaintext | Missing }` —
  pure inspection for `status` (does not require the secret).

### Changes: `main.rs` (CLI)

- `init`: resolve op-passphrase from `--passphrase` / `CAIRN_KEY_PASSPHRASE` / interactive prompt;
  refuse if none unless `--insecure-plaintext`. Always `generate_recovery_code()` and **print it once**
  with a write-it-down warning. Seal, then provision as today.
- `serve` / `run` / `pair-offer` / `pair-accept` / `unpeer`: load the key with the op-passphrase
  (`CAIRN_KEY_PASSPHRASE` / prompt). If the file is sealed and no secret is available, fail with a
  legible error (`key is sealed: set CAIRN_KEY_PASSPHRASE`), never a panic.
- New `seal-key`: migrate a plaintext key → sealed; prompts for op-passphrase, prints a fresh recovery
  code.

### Changes: `identity.rs` (`status`)

- `Status.key_at_rest` → `SEALED (argon2id; dual-recipient)` | `PLAINTEXT (0600; run seal-key)` |
  `MISSING`.
- `Status.dr_escrow` → `recovery code set (off-node escrow)` when the sealed file carries a recovery
  wrap, else the existing `STUBBED …`.
- Add `Status.recovery_escrow: bool` for machine-readable assertions in tests.

## Error handling

- Wrong/absent secret on a sealed key → typed `KeystoreError`, surfaced as a legible CLI error; the
  daemon exits non-zero rather than running keyless.
- Tampered/bit-rotted sealed file → `unseal` returns `None` → same legible "cannot unseal" error
  (indistinguishable from wrong secret by design; we do not leak which recipient/why).
- `init` on an existing key file → refuse (no clobber), as today.

## Testing (TDD — failing tests first)

**`seal.rs` unit tests (no DB):**
- seal → unseal roundtrip via the **operational** secret returns the original seed.
- seal → unseal roundtrip via the **recovery** secret returns the original seed.
- wrong secret → `None`.
- each tampered field (seed_ct, wrap_op.ct, wrap_rec.ct, a salt, a nonce) → `None`.
- `to_cbor`/`from_cbor` roundtrip; magic header present; truncated/garbage bytes → `Err`.
- `base32_encode`/`decode` roundtrip on random vectors; reject invalid chars.
- `generate_recovery_code` returns the expected length/grouping and decodes to 160 bits; two calls
  differ (entropy smoke test).

**`keystore` tests (tmpfile, no DB):**
- `generate_and_seal` then `load(Some(op))` and `load(Some(rec))` both recover the same key.
- `load(None)` on a sealed file → `Err` (legible).
- auto-detect: a raw-32-byte plaintext file loads with `load(None)`; `key_at_rest_state` classifies
  both correctly.
- `seal_existing` migrates a plaintext file; afterwards both secrets unseal and plaintext load fails.

**Node integration test (DB-gated, mirrors existing `tests/` patterns + advisory-lock serialization),
new `tests/keystore_seal.rs`:**
- `init` with an op-passphrase writes a sealed key, returns a recovery code that **independently**
  unseals it; `status` reports `SEALED` and `recovery_escrow = true`.
- an authoring path (e.g. `pair`/`unpeer` or a direct author call) works when the passphrase is
  supplied and fails legibly when it is not.

All existing node tests must stay green (they pass `None`/plaintext today — covered by auto-detect; any
that `init` a node will be updated to pass a test passphrase or `--insecure-plaintext`).

## Dependencies to add (`cairn-node/Cargo.toml`)

- `argon2 = "0.5"` (Argon2id) — MIT/Apache-2.0
- `chacha20poly1305 = "0.10"` (XChaCha20-Poly1305) — MIT/Apache-2.0
- `ciborium = "0.2"` (already used in `cairn-event`) for the bundle encoding
- `getrandom` (already transitive) for salts/nonces/DEK/recovery entropy
- `rpassword` (or std prompt) for the interactive passphrase prompt — confirm MIT/Apache before adding;
  if license is unclear, fall back to env-only + a plain stdin read (no extra dep).

All RustCrypto additions are consistent with issue #11's RustCrypto-dedup direction.

## Explicit non-goals (deferred; named in code + HANDOVER)

- The full sealed **local-state export** bundling node config + draft/scratchpad store (ADR-0026 point
  3 beyond the signing key).
- **Backup-as-cold-peer** and backup-health surfacing (slice B; ADR points 2 & 7).
- **Key rotation / `supersede`** and new-identity-on-recovery (slice C; ADR point 4).
- **Shamir M-of-N** escrow split, **QR** rendering, **OS-keyring/TPM/Secure-Enclave** operational
  secret, an **audited escrow-generation event**.
- The full **restore orchestrator** with shred-replay (slice D; ADR points 1 & 6).

## Why this is correct per the principles

- **Paper-parity (3):** a paper chart survives the computer dying; this gives the signing key an
  off-node survival path (the recovery code in the practice safe) without a confirmation-dialog crutch.
- **Acknowledged uncertainty (4) / honest assembly:** the loss is named, not hidden — lose *both* the
  passphrase and the recovery code and the node is gone; `status` and the `init` warning say so plainly.
- **Anti-capture (7):** the floor is a printed code in a safe — no cloud, no vendor, no mandatory
  hardware. Hardware tokens / quorum recovery are documented opt-ups, not requirements.
- **Defect blast radius (§9):** the seal primitives are the small safety-critical surface — Rust, pure,
  reviewer-legible, exhaustively unit-tested.
