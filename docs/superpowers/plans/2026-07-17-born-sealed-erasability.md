# Born-Sealed Clinical Bodies (ADR-0052 + walking-skeleton seal slice) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ratify the seal-by-default posture (ADR-0052) and build the walking-skeleton seal slice: every clinical JSONB body sealed at write under a per-event DEK (node custody = erasability, not confidentiality), wrapped-DEK custody sidecar on the clinical sync wire, crypto-shred with derived-plaintext scrub, and arrival-order-independent shred propagation.

**Architecture:** Seal-then-sign (signature covers ciphertext; twin inside the sealed region under the same DEK). Wrapped DEKs live in a mutable `event_dek` table beside the log, never inside signed bytes; the node's X25519 unwrap secret is HKDF-derived from the existing Ed25519 seed so the ADR-0026 escrow covers it with no new ceremony. Projections keep firing off `event_log` triggers, but read the clear payload through a `cairn_clear_payload()` helper backed by a mutable `event_clear` shadow table the doors populate — the single derived-plaintext surface a shred scrubs.

**Tech Stack:** Rust (cairn-event / cairn-node / cairn-sync), PL/pgSQL + pgrx (`cairn_pgx`), XChaCha20-Poly1305, X25519 + HKDF-SHA256, PostgreSQL 18.

**Design doc:** `docs/superpowers/specs/2026-07-17-born-sealed-erasability-design.md` (approved 2026-07-17). Issues: #189 (C1) + #92.

## Global Constraints

- **AGPL-3.0 compatibility gate:** new deps `chacha20poly1305` (MIT/Apache-2.0), `x25519-dalek` (BSD-3-Clause), `hkdf` (MIT/Apache-2.0) — all compatible; CI `cargo-deny` must stay green.
- **TDD:** failing test first for every code change; no production code without a driving test.
- **House rule 3:** junior-legible inline docs on every non-trivial function (why + how it fits).
- **House rule 6:** NEVER write key/nonce/seed byte literals in tests or benches — derive at runtime (`std::array::from_fn(|i| …)`).
- **Migration replay rules (memory):** `db/*.sql` files replay on EVERY connect in order; all DDL idempotent (`IF NOT EXISTS`, `CREATE OR REPLACE`, `DROP TRIGGER IF EXISTS` before `CREATE TRIGGER`); never widen an earlier `CREATE OR REPLACE VIEW` from a later file; changing a door function's signature requires `DROP FUNCTION IF EXISTS <old signature>` in the SAME file that recreates it (a `CREATE OR REPLACE` with new args OVERLOADS instead of replacing → ambiguous-call errors).
- **Twin-registry two mirrors (memory):** registering `erasure.shred.asserted` bumps the registry count 18→19 in BOTH `crates/cairn-node/tests/twin_registry.rs` AND `db/tests/034_twin_registry_test.sql` — the SQL mirror is not run by cargo test and drifts silently.
- **Whole workspace green before any commit** (`cargo test --workspace`, currently 700/0), plus `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- **Test env:** DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx` installed); multi-node suites also need `CAIRN_TEST_PG2`/`CAIRN_TEST_PG3` (else they self-skip locally; CI sets all three).
- **cairn_pgx rebuild:** any `extensions/cairn_pgx` change requires `cargo pgrx install` from inside `extensions/cairn_pgx/` (Mac: `SDKROOT` isysroot + `-Wl,-no_fixup_chains` gotcha, see memory `cairn-pgx-build-updated-xcode`); there is no ALTER EXTENSION path — DROP CASCADE + CREATE in the test DB.
- **Docs build:** `uv run --with-requirements docs/requirements.txt -- mkdocs build` must pass (pinned requirements only).
- **Branch:** `feat/adr-0052-born-sealed-189-92` (already created, design doc committed).
- **Pre-production caveat (deliberate):** pre-ADR-0052 plaintext clinical events on dev/PoC rigs now REFUSE at the strict door — wipe dev rigs, never sync them through (ADR-0051 precedent).

---

### Task 1: ADR-0052 + spec prose + docs build

**Files:**
- Create: `docs/spec/decisions/0052-born-sealed-clinical-bodies.md`
- Modify: `docs/spec/data-model.md` (§3.5 bullet at line ~57, §3.8 first bullet at line ~119, §3.14 descriptor note)
- Modify: `docs/spec/identity.md` (§5.9 — safety-projection erasure semantics)
- Modify: `docs/spec/index.md` (spec version → v0.53; add ADR-0052 to any in-file ADR list if present)

**Interfaces:**
- Produces: the ratified vocabulary every later task cites — **erasable** vs **sequestered**, the sealed-payload container shape `{"sealed": true, "alg": "xchacha20poly1305", "nonce": "<hex 24B>", "ct": "<hex>"}`, the twin-under-same-DEK rule, the `event_dek`/`event_clear`/`erasure_shred_log` custody-plane names, and the clinical.\*-must-seal strict-door floor.

- [ ] **Step 1: Write ADR-0052**

Follow the house ADR format (Status/Date/Supersedes header, Context, Decision, Consequences — model on `docs/spec/decisions/0051-*.md`). Content = the design doc's "ADR-0052 content" section, restated as a decision record. Must include, verbatim as normative statements:

1. The erasable/sequestered split (refines ADR-0005 §2 "off by default" → **born-sealed under node custody is the shipped default**; sequester = the same DEK, custody narrowed).
2. Scope: clinical JSONB bodies only; demographic/identity/node-plane/**erasure-plane** events plaintext by necessity (the shred tombstone must outlive all keys). Blob bytes inherit via ADR-0013's per-blob DEK (deferred to the blob-tier slice).
3. Seal-then-sign; the container shape above; AAD binds `event_id`; the legibility twin travels inside the sealed region under the same DEK; the outer `plaintext_twin` is a signed mechanical stub. A sealed twin must never be materialized into any plaintext index.
4. Custody plane: wrapped DEKs in a mutable table beside the log, never inside signed bytes; the `db/001` `event_log.dek_wrapped` column is retired unused (append-only rows can't hold rotating custody); node unwrap key = X25519 derived from the Ed25519 seed via HKDF (`info = "cairn-node-unwrap-x25519-v1"`) so ADR-0026 escrow covers it; DB holds only the public half — ordinary DB backups can never reconstruct a DEK.
5. The strict door refuses an UNSEALED `clinical.*` body (the floor makes the posture unbypassable — principle 12); the apply door admits foreign plaintext leniently (set-union). Strict door refuses a sealed event without its DEK; apply door admits sealed-without-custody on structural checks only.
6. Shred (rung 3) = destroy `event_dek` rows + scrub derived plaintext (`event_clear`, projections, any FTS/RAG index — mandatory invalidation) + the signed `erasure.shred.asserted` tombstone; the log row is never touched; restore/re-sync replays the shred log before projecting; custody is never granted to an already-shredded event (arrival-order independence).
7. Prose closures: safety projection **coarsens-but-survives** rung-3 shred (Rh case), shreddable only at rung 4, named in the honest ceiling; public attachment descriptor is graded (precise descriptor under seal); rung-2 deniable deletion requires born-pseudonymous episodes + abstracted routing, never retroactive.
8. Deferred, named: per-episode DEK hierarchy (measure via the bench), shred authorization policy hooks, sequester/custody-narrowing implementation, blob-byte sealing, unwrap-key rotation.

- [ ] **Step 2: Spec prose edits**

- `data-model.md` §3.5 (line ~57): change "per-record encryption is **off by default**" to the born-sealed default with a pointer to ADR-0052; add the twin-under-seal sentence.
- `data-model.md` §3.8 (line ~119): update the first bullet ("it is **off by default**…") to the erasable/sequestered split; add the FTS/RAG-invalidation-on-shred sentence to the crypto-shred bullet.
- `data-model.md` §3.14: one sentence — the public descriptor is graded; the precise descriptor lives under the seal (ADR-0052).
- `identity.md` §5.9: one short paragraph — the safety projection's erasure semantics (coarsen-but-survive rung 3; rung 4 only; ceiling disclosure).
- `index.md`: spec version v0.52 → v0.53 with a one-line ADR-0052 entry in the version line's style.

- [ ] **Step 3: Build docs**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: clean build, no broken-link warnings for the new ADR.

- [ ] **Step 4: Commit**

```bash
git add docs/spec
git commit -m "docs(#189,#92): ADR-0052 — born-sealed clinical bodies; spec v0.53"
```

---

### Task 2: cairn-event body-seal module (AEAD core)

**Files:**
- Create: `crates/cairn-event/src/seal.rs`
- Modify: `crates/cairn-event/Cargo.toml` (add deps), `crates/cairn-event/src/lib.rs` (add `pub mod seal;`)
- Test: unit tests inside `seal.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `EventError` from `lib.rs` (add variant if needed — see Step 3), `serde_json::Value`.
- Produces (used by Tasks 3–5, 10–12):
  - `pub const SEAL_ALG: &str = "xchacha20poly1305"`
  - `pub fn seal_stub_twin(event_type: &str) -> String`
  - `pub fn seal_event_payload(payload: &serde_json::Value, twin: &str, event_id: &str) -> Result<(serde_json::Value, Zeroizing<[u8; 32]>), EventError>` — returns (container, dek)
  - `pub fn unseal_event_payload(container: &serde_json::Value, dek: &[u8; 32], event_id: &str) -> Result<(serde_json::Value, String), EventError>` — returns (clear payload, clear twin)
  - `pub fn is_sealed_container(payload: &serde_json::Value) -> bool`

- [ ] **Step 1: Add dependencies (license-checked)**

In `crates/cairn-event/Cargo.toml` `[dependencies]` add:

```toml
# ADR-0052 born-sealed bodies: AEAD seal + DEK wrap plane. Licenses checked
# (house rule 1): chacha20poly1305 MIT/Apache-2.0, x25519-dalek BSD-3-Clause,
# hkdf MIT/Apache-2.0, zeroize MIT/Apache-2.0 — all AGPL-3.0-compatible.
chacha20poly1305 = "0.10"
x25519-dalek = { version = "2", features = ["static_secrets"] }
hkdf = "0.12"
zeroize = "1"
```

Run: `cargo tree -p cairn-event | head -30` to confirm resolution; `cargo deny check licenses` if installed locally (CI enforces regardless).

- [ ] **Step 2: Write the failing tests**

Create `crates/cairn-event/src/seal.rs` with module doc + tests first (house rule 6 — keys derived, never literal):

```rust
//! ADR-0052 born-sealed clinical bodies: the AEAD seal core.
//!
//! WHY THIS EXISTS: every clinical JSONB body is sealed at write under a
//! per-event DEK so the ADR-0005 erasure ladder stays reachable forever
//! (erasability, not confidentiality — the node holds custody by default).
//! Seal-then-sign: the author calls seal_event_payload BEFORE cairn_event::sign,
//! so the signature covers the ciphertext and still verifies after a shred.
//! The legibility twin travels INSIDE the sealed region under the same DEK
//! (#92 collision (a)); the outer plaintext_twin is a signed mechanical stub.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dek_fixture() -> [u8; 32] {
        // House rule 6: derived, never a literal.
        std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3))
    }

    #[test]
    fn seal_then_unseal_round_trips_payload_and_twin() {
        let payload = json!({"medication_id": "m", "substance": {"term": "amoxicillin"}});
        let twin = "amoxicillin 500 mg — patient reports taking";
        let (container, dek) = seal_event_payload(&payload, twin, "evt-1").unwrap();
        assert_eq!(container["sealed"], json!(true));
        assert_eq!(container["alg"], json!(SEAL_ALG));
        // No plaintext leaks into the container.
        let ct_json = container.to_string();
        assert!(!ct_json.contains("amoxicillin"));
        let (p2, t2) = unseal_event_payload(&container, &dek, "evt-1").unwrap();
        assert_eq!(p2, payload);
        assert_eq!(t2, twin);
    }

    #[test]
    fn unseal_fails_with_wrong_dek() {
        let (container, _dek) = seal_event_payload(&json!({"a": 1}), "t", "evt-1").unwrap();
        let wrong = dek_fixture();
        assert!(unseal_event_payload(&container, &wrong, "evt-1").is_err());
    }

    #[test]
    fn unseal_fails_when_event_id_differs_aad_binding() {
        // The AAD binds the container to its event: a ciphertext transplanted
        // onto another event id must not open (defense in depth beside the sig).
        let (container, dek) = seal_event_payload(&json!({"a": 1}), "t", "evt-1").unwrap();
        assert!(unseal_event_payload(&container, &dek, "evt-2").is_err());
    }

    #[test]
    fn two_seals_of_same_payload_differ_fresh_dek_and_nonce() {
        let p = json!({"a": 1});
        let (c1, d1) = seal_event_payload(&p, "t", "e").unwrap();
        let (c2, d2) = seal_event_payload(&p, "t", "e").unwrap();
        assert_ne!(c1["ct"], c2["ct"]);
        assert_ne!(d1.as_slice(), d2.as_slice());
    }

    #[test]
    fn is_sealed_container_detects_shape() {
        let (c, _d) = seal_event_payload(&json!({}), "t", "e").unwrap();
        assert!(is_sealed_container(&c));
        assert!(!is_sealed_container(&json!({"medication_id": "m"})));
        assert!(!is_sealed_container(&json!({"sealed": false})));
    }

    #[test]
    fn stub_twin_names_type_and_seal_state() {
        let s = seal_stub_twin("clinical.medication.asserted");
        assert!(s.contains("clinical.medication.asserted"));
        assert!(s.contains("seal"));
    }
}
```

Add `pub mod seal;` to `lib.rs` next to the other module declarations (lib.rs:33-40).

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p cairn-event seal -- --nocapture`
Expected: compile FAILURE (functions not defined).

- [ ] **Step 4: Implement**

```rust
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::EventError;

/// The one sealed-body AEAD algorithm (crypto-agile: the container names it,
/// additive evolution adds members, never reinterprets this one).
pub const SEAL_ALG: &str = "xchacha20poly1305";
/// AAD domain tag: binds a container to the seal plane AND its event id, so a
/// ciphertext cannot be transplanted between events even pre-signature-check.
const SEAL_AAD_CONTEXT: &[u8] = b"cairn-sealed-body-v1";

/// Mechanical outer twin for a sealed event (principle 11: the row stays
/// honestly self-describing as WHAT it is; the real twin is under the seal).
pub fn seal_stub_twin(event_type: &str) -> String {
    format!("sealed {event_type} event — twin under seal (ADR-0052)")
}

/// True iff a payload value is the ADR-0052 sealed container shape.
pub fn is_sealed_container(payload: &serde_json::Value) -> bool {
    payload.get("sealed").and_then(|v| v.as_bool()) == Some(true)
}

fn aad_for(event_id: &str) -> Vec<u8> {
    let mut aad = SEAL_AAD_CONTEXT.to_vec();
    aad.extend_from_slice(event_id.as_bytes());
    aad
}

/// Seal a clear payload + its clear twin under a FRESH per-event DEK.
/// Returns (container, dek). The caller places the container into
/// EventBody.payload and seal_stub_twin(..) into EventBody.plaintext_twin,
/// then signs — seal-then-sign, so the signature covers the ciphertext.
pub fn seal_event_payload(
    payload: &serde_json::Value,
    twin: &str,
    event_id: &str,
) -> Result<(serde_json::Value, Zeroizing<[u8; 32]>), EventError> {
    // Fresh DEK + nonce from the OS RNG (production key material is always
    // random — house rule 6 applies to tests only).
    let mut dek = Zeroizing::new([0u8; 32]);
    getrandom::fill(dek.as_mut()).map_err(|_| EventError::Seal("entropy failure".into()))?;
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| EventError::Seal("entropy failure".into()))?;

    // The inner (sealed) region: clear payload AND clear twin together, so the
    // twin is under the SAME DEK as its body (#92 (a), normative in ADR-0052).
    let inner = serde_json::json!({ "payload": payload, "plaintext_twin": twin });
    let inner_bytes = serde_json::to_vec(&inner)
        .map_err(|e| EventError::Seal(format!("inner serialize: {e}")))?;

    let cipher = XChaCha20Poly1305::new(Key::from_slice(dek.as_ref()));
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce),
                 Payload { msg: &inner_bytes, aad: &aad_for(event_id) })
        .map_err(|_| EventError::Seal("encrypt failure".into()))?;

    let container = serde_json::json!({
        "sealed": true,
        "alg": SEAL_ALG,
        "nonce": hex::encode(nonce),
        "ct": hex::encode(ct),
    });
    Ok((container, dek))
}

/// Open a sealed container with its DEK. Returns (clear payload, clear twin).
/// Errors on wrong DEK, wrong event id (AAD), unknown alg, or malformed shape
/// — every failure is a refusal, never a silent fallback.
pub fn unseal_event_payload(
    container: &serde_json::Value,
    dek: &[u8; 32],
    event_id: &str,
) -> Result<(serde_json::Value, String), EventError> {
    if !is_sealed_container(container) {
        return Err(EventError::Seal("not a sealed container".into()));
    }
    let alg = container.get("alg").and_then(|v| v.as_str()).unwrap_or("");
    if alg != SEAL_ALG {
        return Err(EventError::Seal(format!("unknown seal alg {alg:?}")));
    }
    let nonce = hex::decode(container.get("nonce").and_then(|v| v.as_str()).unwrap_or(""))
        .map_err(|_| EventError::Seal("malformed nonce hex".into()))?;
    let ct = hex::decode(container.get("ct").and_then(|v| v.as_str()).unwrap_or(""))
        .map_err(|_| EventError::Seal("malformed ct hex".into()))?;
    if nonce.len() != 24 {
        return Err(EventError::Seal("nonce must be 24 bytes".into()));
    }
    let cipher = XChaCha20Poly1305::new(Key::from_slice(dek));
    let inner_bytes = cipher
        .decrypt(XNonce::from_slice(&nonce),
                 Payload { msg: ct.as_slice(), aad: &aad_for(event_id) })
        .map_err(|_| EventError::Seal("AEAD open failed (wrong DEK or tampered)".into()))?;
    let inner: serde_json::Value = serde_json::from_slice(&inner_bytes)
        .map_err(|e| EventError::Seal(format!("inner parse: {e}")))?;
    let payload = inner.get("payload").cloned()
        .ok_or_else(|| EventError::Seal("inner missing payload".into()))?;
    let twin = inner.get("plaintext_twin").and_then(|v| v.as_str())
        .ok_or_else(|| EventError::Seal("inner missing plaintext_twin".into()))?
        .to_string();
    Ok((payload, twin))
}
```

Add to `lib.rs`'s `EventError` enum a variant `Seal(String)` following the existing variants' style (with its `Display` arm). Check `hex` is already a cairn-event dep — it is used by cairn_pgx; if absent from cairn-event add `hex = "0.4"` (MIT/Apache-2.0 ✓). Note `getrandom = "0.4"` API is `getrandom::fill` — match whatever the existing cairn-event call sites use (grep `getrandom::` and mirror).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cairn-event seal`
Expected: 6 passed.

- [ ] **Step 6: fmt/clippy + commit**

```bash
cargo fmt && cargo clippy -p cairn-event --all-targets -- -D warnings
git add crates/cairn-event
git commit -m "feat(#189): cairn-event seal core — per-event DEK AEAD, twin under the same seal"
```

---

### Task 3: cairn-event DEK wrap plane (X25519 + HKDF + unwrap-key cert)

**Files:**
- Modify: `crates/cairn-event/src/seal.rs` (append), `crates/cairn-event/src/lib.rs` (new signing context + cert helpers)
- Test: unit tests in `seal.rs` + `lib.rs`

**Interfaces:**
- Consumes: `SigningKey`/`VerifyingKey` re-exports, `cose_sign1_in_context` (private in lib.rs — the cert helpers live in lib.rs so they can reach it), `SigningContext` (lib.rs:90-100).
- Produces (used by Tasks 4, 5, 10, 11, 12):
  - `pub fn derive_unwrap_secret(seed: &[u8; 32]) -> Zeroizing<[u8; 32]>`
  - `pub fn unwrap_public(unwrap_secret: &[u8; 32]) -> [u8; 32]`
  - `pub fn wrap_dek_for(dek: &[u8; 32], recipient_pub: &[u8; 32]) -> Result<Vec<u8>, EventError>` — 104-byte blob `eph_pub(32) ‖ nonce(24) ‖ ct(48)`
  - `pub fn unwrap_dek(wrapped: &[u8], unwrap_secret: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, EventError>`
  - In lib.rs: `pub const CTX_UNWRAP_KEY: SigningContext` (`"application/cairn-unwrap-key+cbor"`), `pub fn sign_unwrap_key_cert(sk: &SigningKey, x25519_pub: &[u8; 32]) -> Result<Vec<u8>, EventError>`, `pub fn verify_unwrap_key_cert(bytes: &[u8]) -> Result<(String, [u8; 32]), EventError>` — returns (signer hex kid, x25519 pub)

- [ ] **Step 1: Write the failing tests**

Append to `seal.rs` tests:

```rust
    fn seed_fixture(tag: u8) -> [u8; 32] {
        std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(tag))
    }

    #[test]
    fn wrap_then_unwrap_round_trips_the_dek() {
        let seed = seed_fixture(1);
        let secret = derive_unwrap_secret(&seed);
        let public = unwrap_public(&secret);
        let dek = dek_fixture();
        let wrapped = wrap_dek_for(&dek, &public).unwrap();
        assert_eq!(wrapped.len(), 32 + 24 + 32 + 16); // eph ‖ nonce ‖ ct+tag
        let opened = unwrap_dek(&wrapped, &secret).unwrap();
        assert_eq!(opened.as_slice(), &dek);
    }

    #[test]
    fn unwrap_fails_for_the_wrong_recipient() {
        let s_a = derive_unwrap_secret(&seed_fixture(1));
        let s_b = derive_unwrap_secret(&seed_fixture(2));
        let wrapped = wrap_dek_for(&dek_fixture(), &unwrap_public(&s_a)).unwrap();
        assert!(unwrap_dek(&wrapped, &s_b).is_err());
    }

    #[test]
    fn derivation_is_deterministic_and_domain_separated_from_signing() {
        let seed = seed_fixture(1);
        let a = derive_unwrap_secret(&seed);
        let b = derive_unwrap_secret(&seed);
        assert_eq!(a.as_slice(), b.as_slice());
        assert_ne!(a.as_slice(), &seed); // never the raw signing seed
    }
```

And in `lib.rs` tests (near the existing signing-context tests):

```rust
    #[test]
    fn unwrap_key_cert_round_trips_and_binds_signer() {
        let sk = generate_key();
        let xpub: [u8; 32] = std::array::from_fn(|i| i as u8);
        let cert = sign_unwrap_key_cert(&sk, &xpub).unwrap();
        let (kid, got) = verify_unwrap_key_cert(&cert).unwrap();
        assert_eq!(kid, hex::encode(sk.verifying_key().to_bytes()));
        assert_eq!(got, xpub);
    }

    #[test]
    fn unwrap_key_cert_rejects_event_context_tokens() {
        // ADR-0040 domain separation: an ordinary event signed blob must not
        // verify as an unwrap-key cert.
        let sk = generate_key();
        let body = minimal_test_body(&sk); // reuse the existing test-body helper in lib.rs tests
        let ev = sign(&body, &sk).unwrap();
        assert!(verify_unwrap_key_cert(&ev.signed_bytes).is_err());
    }
```

(If `lib.rs` tests have no `minimal_test_body` helper, reuse whatever existing helper builds an `EventBody` for the sign/verify tests there — mirror its name.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cairn-event`
Expected: compile FAILURE (functions not defined).

- [ ] **Step 3: Implement the wrap plane in seal.rs**

```rust
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

/// HKDF domain tag for deriving the node's X25519 unwrap secret from its
/// Ed25519 seed. One master secret, two INDEPENDENT keys (signing vs unwrap)
/// — so the existing ADR-0026 seal/recovery escrow covers DEK custody with
/// no new ceremony, and a DB backup (public half only) can never unwrap.
const UNWRAP_KEY_HKDF_INFO: &[u8] = b"cairn-node-unwrap-x25519-v1";
/// KEK-derivation + AEAD domain tag for the DEK wrap itself.
const WRAP_AAD_CONTEXT: &[u8] = b"cairn-dek-wrap-v1";

pub fn derive_unwrap_secret(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, seed);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(UNWRAP_KEY_HKDF_INFO, out.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

pub fn unwrap_public(unwrap_secret: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*unwrap_secret)).to_bytes()
}

fn wrap_kek(shared: &[u8], eph_pub: &[u8; 32], recipient_pub: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    // Salt binds both public halves so a KEK is unique to this (eph, recipient)
    // pair; info is the domain tag.
    let mut salt = Vec::with_capacity(64);
    salt.extend_from_slice(eph_pub);
    salt.extend_from_slice(recipient_pub);
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(WRAP_AAD_CONTEXT, out.as_mut()).expect("valid length");
    out
}

/// ECIES-style wrap: fresh ephemeral X25519 → DH with the recipient's public
/// half → HKDF KEK → XChaCha20-Poly1305 over the DEK. Only the recipient's
/// SECRET half (held in the daemon, never the DB) can unwrap.
pub fn wrap_dek_for(dek: &[u8; 32], recipient_pub: &[u8; 32]) -> Result<Vec<u8>, EventError> {
    let mut eph_bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(eph_bytes.as_mut()).map_err(|_| EventError::Seal("entropy failure".into()))?;
    let eph = StaticSecret::from(*eph_bytes.as_ref());
    let eph_pub = PublicKey::from(&eph).to_bytes();
    let shared = eph.diffie_hellman(&PublicKey::from(*recipient_pub));
    let kek = wrap_kek(shared.as_bytes(), &eph_pub, recipient_pub);
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| EventError::Seal("entropy failure".into()))?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(kek.as_ref()));
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: dek.as_slice(), aad: WRAP_AAD_CONTEXT })
        .map_err(|_| EventError::Seal("wrap encrypt failure".into()))?;
    let mut out = Vec::with_capacity(32 + 24 + ct.len());
    out.extend_from_slice(&eph_pub);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn unwrap_dek(wrapped: &[u8], unwrap_secret: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, EventError> {
    if wrapped.len() != 32 + 24 + 32 + 16 {
        return Err(EventError::Seal("malformed wrapped DEK length".into()));
    }
    let eph_pub: [u8; 32] = wrapped[..32].try_into().expect("sliced 32");
    let nonce = &wrapped[32..56];
    let ct = &wrapped[56..];
    let me = StaticSecret::from(*unwrap_secret);
    let my_pub = PublicKey::from(&me).to_bytes();
    let shared = me.diffie_hellman(&PublicKey::from(eph_pub));
    let kek = wrap_kek(shared.as_bytes(), &eph_pub, &my_pub);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(kek.as_ref()));
    let pt = cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: WRAP_AAD_CONTEXT })
        .map_err(|_| EventError::Seal("wrap open failed (wrong recipient or tampered)".into()))?;
    let mut dek = Zeroizing::new([0u8; 32]);
    dek.copy_from_slice(&pt);
    Ok(dek)
}
```

- [ ] **Step 4: Implement the unwrap-key cert in lib.rs**

Next to the other contexts (lib.rs:105-109) add `pub const CTX_UNWRAP_KEY: SigningContext = SigningContext("application/cairn-unwrap-key+cbor");`. Then, near `sign`/the attestation helpers:

```rust
/// A node's signed unwrap-key certificate: binds its X25519 public unwrap key
/// to its Ed25519 identity, in its own ADR-0040 signing context so it can
/// never be replayed as an event or attestation. CBOR payload:
/// {"kid": <hex ed25519 pub>, "x25519_pub": <32 bytes>}.
pub fn sign_unwrap_key_cert(sk: &SigningKey, x25519_pub: &[u8; 32]) -> Result<Vec<u8>, EventError> {
    let payload = serde_json::json!({
        "kid": hex::encode(sk.verifying_key().to_bytes()),
        "x25519_pub": hex::encode(x25519_pub),
    });
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes)
        .map_err(|e| EventError::Seal(format!("cert serialize: {e}")))?;
    cose_sign1_in_context(bytes, sk, CTX_UNWRAP_KEY)
}

/// Verify a cert and return (signer hex kid, X25519 public key). The signature
/// key IS the kid — the payload's kid field must match it (no third-party
/// binding).
pub fn verify_unwrap_key_cert(bytes: &[u8]) -> Result<(String, [u8; 32]), EventError> {
    // Mirror the existing verify path (cose_verify1_parsed) with CTX_UNWRAP_KEY,
    // extract the payload, check payload.kid == signer kid, hex-decode
    // x25519_pub into [u8; 32]. Follow the shape of the attestation verify
    // helper already in this file.
    ...
}
```

(Implementer: model `verify_unwrap_key_cert` line-by-line on the existing attestation-token verify helper in lib.rs — same COSE parse, same context enforcement, then the two field extractions with legible `EventError`s.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cairn-event`
Expected: all pass (existing + 5 new).

- [ ] **Step 6: fmt/clippy + commit**

```bash
cargo fmt && cargo clippy -p cairn-event --all-targets -- -D warnings
git add crates/cairn-event
git commit -m "feat(#189): DEK wrap plane — X25519/HKDF derive-from-seed, ECIES wrap, unwrap-key cert (CTX_UNWRAP_KEY)"
```

---

### Task 4: cairn_pgx in-DB unseal + wrap

**Files:**
- Modify: `extensions/cairn_pgx/src/lib.rs`, `extensions/cairn_pgx/Cargo.toml` (no new deps — everything comes via the `cairn-event` path dependency)

**Interfaces:**
- Consumes: `cairn_event::seal::{unseal_event_payload, wrap_dek_for, is_sealed_container}`.
- Produces (SQL, used by Tasks 5–9):
  - `cairn_unseal_body(container jsonb, dek bytea, event_id text) RETURNS jsonb` — the inner `{"payload":…, "plaintext_twin":…}` object, or NULL on any failure (the door raises the legible error; NULL keeps the pgrx surface panic-free)
  - `cairn_wrap_dek(dek bytea, unwrap_pub bytea) RETURNS bytea` — VOLATILE (fresh ephemeral + nonce)

- [ ] **Step 1: Write the failing pg_test**

In `extensions/cairn_pgx/src/lib.rs` tests module (follow the existing `#[pg_test]` style):

```rust
    #[pg_test]
    fn unseal_body_round_trips_via_sql() {
        // Seal in Rust, open via the SQL surface — the exact door flow.
        let payload = serde_json::json!({"medication_id": "m1"});
        let (container, dek) =
            cairn_event::seal::seal_event_payload(&payload, "the twin", "evt-9").unwrap();
        let inner = Spi::get_one_with_args::<pgrx::JsonB>(
            "SELECT cairn_unseal_body($1::jsonb, $2, $3)",
            &[container.to_string().into(), dek.as_slice().into(), "evt-9".into()],
        ).unwrap().unwrap();
        assert_eq!(inner.0["plaintext_twin"], serde_json::json!("the twin"));
        assert_eq!(inner.0["payload"]["medication_id"], serde_json::json!("m1"));
    }

    #[pg_test]
    fn unseal_body_returns_null_on_wrong_dek() {
        let (container, _dek) =
            cairn_event::seal::seal_event_payload(&serde_json::json!({}), "t", "e").unwrap();
        let wrong: [u8; 32] = std::array::from_fn(|i| i as u8);
        let got = Spi::get_one_with_args::<pgrx::JsonB>(
            "SELECT cairn_unseal_body($1::jsonb, $2, $3)",
            &[container.to_string().into(), wrong.as_slice().into(), "e".into()],
        ).unwrap();
        assert!(got.is_none());
    }

    #[pg_test]
    fn wrap_dek_produces_openable_custody() {
        let seed: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_add(9));
        let secret = cairn_event::seal::derive_unwrap_secret(&seed);
        let public = cairn_event::seal::unwrap_public(&secret);
        let dek: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(3));
        let wrapped = Spi::get_one_with_args::<Vec<u8>>(
            "SELECT cairn_wrap_dek($1, $2)",
            &[dek.as_slice().into(), public.as_slice().into()],
        ).unwrap().unwrap();
        let opened = cairn_event::seal::unwrap_dek(&wrapped, &secret).unwrap();
        assert_eq!(opened.as_slice(), &dek);
    }
```

(Match the SPI argument style already used in this file's tests — mirror an existing `get_one_with_args` call exactly; pgrx 0.18 arg-tuple syntax differs across versions.)

- [ ] **Step 2: Run to verify failure**

Run (from `extensions/cairn_pgx/`): `cargo pgrx test pg18`
Expected: compile FAILURE.

- [ ] **Step 3: Implement**

```rust
/// ADR-0052: open a sealed body container with its DEK, in-DB, so the floor
/// checks and projections run on plaintext INSIDE the door (unbypassable —
/// principle 12). Returns NULL on any failure; the door raises the legible
/// refusal. IMMUTABLE: pure function of (container, dek, event_id).
#[pg_extern(immutable, parallel_safe)]
fn cairn_unseal_body(container: pgrx::JsonB, dek: &[u8], event_id: &str) -> Option<pgrx::JsonB> {
    let dek: &[u8; 32] = dek.try_into().ok()?;
    let (payload, twin) =
        cairn_event::seal::unseal_event_payload(&container.0, dek, event_id).ok()?;
    Some(pgrx::JsonB(serde_json::json!({ "payload": payload, "plaintext_twin": twin })))
}

/// ADR-0052: wrap a DEK for a recipient's X25519 public unwrap key. VOLATILE
/// (fresh ephemeral key + nonce per call). The DB only ever sees the PUBLIC
/// half — a DB backup can never reconstruct custody.
#[pg_extern(volatile, parallel_safe)]
fn cairn_wrap_dek(dek: &[u8], unwrap_pub: &[u8]) -> Vec<u8> {
    let dek: &[u8; 32] = dek.try_into()
        .unwrap_or_else(|_| pgrx::error!("cairn_wrap_dek: DEK must be 32 bytes"));
    let unwrap_pub: &[u8; 32] = unwrap_pub.try_into()
        .unwrap_or_else(|_| pgrx::error!("cairn_wrap_dek: unwrap_pub must be 32 bytes"));
    cairn_event::seal::wrap_dek_for(dek, unwrap_pub)
        .unwrap_or_else(|e| pgrx::error!("cairn_wrap_dek: {e}"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run (from `extensions/cairn_pgx/`): `cargo pgrx test pg18`
Expected: PASS.

- [ ] **Step 5: Reinstall the extension into the local test cluster**

Run (from `extensions/cairn_pgx/`): `cargo pgrx install --pg-config $(which pg_config)` (adjust to the local :5532 PG18 pg_config; see memory `pg-test-substrate` / `cairn-pgx-build-updated-xcode` for the Mac SDKROOT gotcha). Then in `cairn_test`: `DROP EXTENSION cairn_pgx CASCADE; CREATE EXTENSION cairn_pgx;` — CASCADE note: the db/*.sql schema replays on next `connect_and_load_schema`, restoring dropped dependents.

- [ ] **Step 6: Commit**

```bash
git add extensions/cairn_pgx
git commit -m "feat(#189): cairn_pgx — in-DB cairn_unseal_body + cairn_wrap_dek (ADR-0052 door surface)"
```

---

### Task 5: db/037 custody-plane schema + erasure.shred registration (mirrors 18→19)

**Files:**
- Create: `db/037_born_sealed.sql`
- Modify: `crates/cairn-node/src/db.rs` (append `include_str!("../../../db/037_born_sealed.sql")` to the `SCHEMA` array, matching the 036 entry's style)
- Modify: `crates/cairn-node/tests/twin_registry.rs` (18→19 + the new expected row)
- Modify: `db/tests/034_twin_registry_test.sql` (18→19 + comment)
- Test: `crates/cairn-node/tests/born_sealed_schema.rs` (new)

**Interfaces:**
- Produces (used by Tasks 6–13): tables `node_unwrap_key(singleton, unwrap_pub)`, `event_dek(event_id, dek_wrapped)`, `event_clear(event_id, body, twin)`, `erasure_shred_log(target_event_id, shred_event_id, basis, shredded_at)`; functions `cairn_register_unwrap_key(bytea)`, `cairn_clear_payload(event_log) RETURNS jsonb`, `cairn_execute_shred(uuid, uuid, text)`, `cairn_check_erasure_shred(text, jsonb)`; event type `erasure.shred.asserted` classified + twin-registered.

- [ ] **Step 1: Write the failing test**

`crates/cairn-node/tests/born_sealed_schema.rs` (copy the header boilerplate — `cs()`, `db_msg()`, `db::test_serial_guard`, `db::connect_and_load_schema` — from `crates/cairn-node/tests/medication.rs:1-45`):

```rust
//! ADR-0052 custody-plane schema: tables exist, are locked down, and the
//! clear-payload helper resolves sealed vs unsealed rows.
//! DB-gated on $CAIRN_TEST_PG, serialized via db::test_serial_guard.

#[tokio::test]
async fn custody_plane_tables_exist_and_are_locked() {
    let Some(conn) = cs() else { return };
    let base = db::connect(&conn).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&conn).await.unwrap();
    for t in ["node_unwrap_key", "event_dek", "event_clear", "erasure_shred_log"] {
        let n: i64 = c.query_one(
            "SELECT count(*) FROM information_schema.tables WHERE table_name = $1", &[&t],
        ).await.unwrap().get(0);
        assert_eq!(n, 1, "table {t} missing");
    }
    // The mutable custody tables are door-managed: cairn_agent has no direct DML.
    for t in ["event_dek", "event_clear", "erasure_shred_log", "node_unwrap_key"] {
        let ok: bool = c.query_one(
            "SELECT has_table_privilege('cairn_agent', $1, 'INSERT')", &[&t],
        ).await.unwrap().get(0);
        assert!(!ok, "cairn_agent must not INSERT into {t} directly");
    }
}

#[tokio::test]
async fn register_unwrap_key_is_idempotent_and_rejects_rotation() {
    let Some(conn) = cs() else { return };
    let base = db::connect(&conn).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&conn).await.unwrap();
    c.execute("DELETE FROM node_unwrap_key", &[]).await.unwrap(); // test reset
    let pub_a: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(5)).collect();
    let pub_b: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(7)).collect();
    c.execute("SELECT cairn_register_unwrap_key($1)", &[&pub_a]).await.unwrap();
    c.execute("SELECT cairn_register_unwrap_key($1)", &[&pub_a]).await.unwrap(); // idempotent
    let err = c.execute("SELECT cairn_register_unwrap_key($1)", &[&pub_b]).await.unwrap_err();
    assert!(db_msg(&err).contains("rotation"), "got: {}", db_msg(&err));
}

#[tokio::test]
async fn erasure_shred_type_is_registered_and_twin_checked() {
    let Some(conn) = cs() else { return };
    let base = db::connect(&conn).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&conn).await.unwrap();
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_type_class WHERE event_type = 'erasure.shred.asserted'", &[],
    ).await.unwrap().get(0);
    assert_eq!(n, 1);
    let n: i64 = c.query_one(
        "SELECT count(*) FROM cairn_event_twin_check WHERE event_type = 'erasure.shred.asserted'", &[],
    ).await.unwrap().get(0);
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test born_sealed_schema`
Expected: FAIL (tables missing — db/037 not yet in the SCHEMA array).

- [ ] **Step 3: Write db/037_born_sealed.sql**

```sql
-- 037_born_sealed.sql — ADR-0052: born-sealed clinical bodies, the custody plane.
--
-- WHY: every clinical JSONB body is sealed at write under a per-event DEK so the
-- ADR-0005 erasure ladder stays reachable forever. These are the MUTABLE tables
-- beside the append-only log: custody may rotate and derived plaintext may be
-- scrubbed, so none of them get the append-only trigger — deliberately.
-- The reserved event_log.dek_wrapped column (db/001) is retired unused: an
-- append-only row cannot hold rotating multi-holder custody.

-- ---------------------------------------------------------------------------
-- 1. The node's X25519 public unwrap key (single row). The SECRET half lives in
--    the daemon keystore (derived from the Ed25519 seed, ADR-0026 escrow) and
--    NEVER enters the database — a DB backup can never reconstruct a DEK.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS node_unwrap_key (
    singleton     BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    unwrap_pub    BYTEA       NOT NULL CHECK (octet_length(unwrap_pub) = 32),
    registered_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE OR REPLACE FUNCTION cairn_register_unwrap_key(p_pub BYTEA) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
DECLARE v_existing BYTEA;
BEGIN
    SELECT unwrap_pub INTO v_existing FROM node_unwrap_key;
    IF v_existing IS NULL THEN
        INSERT INTO node_unwrap_key (unwrap_pub) VALUES (p_pub)
        ON CONFLICT (singleton) DO NOTHING;
    ELSIF v_existing <> p_pub THEN
        -- Unwrap-key rotation re-wraps every custody row — a deliberate,
        -- separate ceremony (ADR-0052 deferred list). Refuse a silent swap.
        RAISE EXCEPTION 'cairn_register_unwrap_key: a different unwrap key is registered — rotation is a separate ceremony (ADR-0052)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 2. Per-event DEK custody (this node's wrapped copy). Destroying a row IS the
--    local half of a crypto-shred.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS event_dek (
    event_id    UUID        PRIMARY KEY,
    dek_wrapped BYTEA       NOT NULL,
    wrapped_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- ---------------------------------------------------------------------------
-- 3. The operational clear view of sealed bodies: THE single derived-plaintext
--    surface (clear payload + clear twin), populated by the doors, deleted by a
--    shred. No FK to event_log: the door inserts this row BEFORE the event_log
--    row so the AFTER INSERT projection triggers can already read it (same
--    transaction — atomicity keeps them consistent). Future FTS/RAG indexes
--    MUST build on this table and nothing else (#92 (b)).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS event_clear (
    event_id UUID  PRIMARY KEY,
    body     JSONB NOT NULL,   -- the CLEAR payload (matches event_log.body semantics)
    twin     TEXT  NOT NULL    -- the CLEAR legibility twin
);

-- ---------------------------------------------------------------------------
-- 4. The shred log: which events have been erased here. Rebuilt idempotently
--    from the append-only log below, so a restore/full-replay re-applies every
--    shred BEFORE any custody or projection could resurrect (§3.8).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS erasure_shred_log (
    target_event_id UUID        PRIMARY KEY,
    shred_event_id  UUID        NOT NULL,
    basis           TEXT        NOT NULL,
    shredded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- ---------------------------------------------------------------------------
-- 5. Projection read helper: the ONE way a projection trigger reads a clinical
--    payload. Unsealed → the derived body column; sealed → the clear shadow
--    (NULL when this node holds no custody: the caller skips projection).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_clear_payload(ev event_log) RETURNS jsonb
LANGUAGE sql STABLE SET search_path = public AS $$
    SELECT CASE WHEN NOT ev.sealed THEN ev.body
                ELSE (SELECT body FROM event_clear WHERE event_id = ev.event_id)
           END
$$;

-- ---------------------------------------------------------------------------
-- 6. erasure.shred.asserted — the rung-3 audited tombstone (plaintext BY DESIGN:
--    it must outlive every key). Classified additive: the erasure arm in the
--    doors owns target handling (a shred may arrive BEFORE its target on the
--    sync wire — the targets_other gate would wrongly reject that at apply).
-- ---------------------------------------------------------------------------
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('erasure.shred.asserted', 'additive', false)
ON CONFLICT (event_type) DO NOTHING;

CREATE OR REPLACE FUNCTION cairn_check_erasure_shred(p_type text, b jsonb) RETURNS void
LANGUAGE plpgsql IMMUTABLE SET search_path = public AS $$
BEGIN
    IF (b -> 'payload' ->> 'target_event_id') IS NULL THEN
        RAISE EXCEPTION 'erasure.shred: payload must name target_event_id (ADR-0052)';
    END IF;
    PERFORM (b -> 'payload' ->> 'target_event_id')::uuid;
    IF COALESCE(b -> 'payload' ->> 'basis', '') = '' THEN
        RAISE EXCEPTION 'erasure.shred: payload must carry a non-empty basis (the audited "why" — ADR-0005 rung 3)';
    END IF;
END;
$$;

INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('erasure.shred.asserted', 'cairn_check_erasure_shred',
     'erasure.shred requires a non-empty authored twin (the tombstone must be legible — ADR-0052)')
ON CONFLICT (event_type) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 7. Shred execution, shared by both doors (never drifts): record, scrub
--    custody + derived plaintext + provenance-precise projection rows. The
--    event_log row is NEVER touched (append-only; signature still verifies).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_execute_shred(p_target uuid, p_shred_event uuid, p_basis text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
DECLARE v_ca BYTEA;
BEGIN
    INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis)
    VALUES (p_target, p_shred_event, p_basis)
    ON CONFLICT (target_event_id) DO NOTHING;

    -- Provenance-precise projection scrub: only rows THIS event produced.
    -- (Overlay winners from other, unshredded events survive — never over-erase.)
    SELECT content_address INTO v_ca FROM event_log WHERE event_id = p_target;
    IF v_ca IS NOT NULL THEN
        DELETE FROM medication_statement WHERE content_address = v_ca;
        DELETE FROM medication_cessation WHERE content_address = v_ca;
    END IF;
    -- The initial-dose seed row is keyed by the assert event's own id (db/032).
    DELETE FROM medication_dose_event WHERE dose_event_id = p_target;

    -- Derived plaintext + custody die last (the scrub above read nothing from
    -- them, so order is safety, not correctness).
    DELETE FROM event_clear WHERE event_id = p_target;
    DELETE FROM event_dek   WHERE event_id = p_target;
END;
$$;

-- Rebuild the shred log from the append-only record on every load (idempotent):
-- this is "restore replays the shred log before projecting" for the wiped-and-
-- reloaded case.
INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis)
SELECT (body ->> 'target_event_id')::uuid, event_id, COALESCE(body ->> 'basis', '(unrecorded)')
FROM event_log WHERE event_type = 'erasure.shred.asserted'
ON CONFLICT (target_event_id) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 8. Grant floor: door-managed only. SELECT for the serve/read paths.
-- ---------------------------------------------------------------------------
REVOKE ALL ON node_unwrap_key, event_dek, event_clear, erasure_shred_log FROM PUBLIC;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_agent') THEN
        REVOKE ALL ON node_unwrap_key, event_dek, event_clear, erasure_shred_log FROM cairn_agent;
        GRANT SELECT ON event_clear TO cairn_agent;  -- the clear READ surface (chart/FTS)
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        GRANT SELECT ON event_dek, erasure_shred_log, node_unwrap_key TO cairn_node;  -- serve-side custody reads
        GRANT EXECUTE ON FUNCTION cairn_register_unwrap_key(bytea) TO cairn_node;
    END IF;
END $$;
```

(Check db/031's medication tables for the exact `content_address` column names before finalizing the scrub arm — `medication_statement.content_address` confirmed at db/031:149; verify `medication_cessation` has one at db/031:216-230 and adjust: if absent, scope the scrub to `medication_statement` + `medication_dose_event` and note it in the ADR's deferred list.)

Also check whether the role names/grant style matches db/036's tail — mirror it.

- [ ] **Step 4: Wire into the loader + update the two registry mirrors**

- `crates/cairn-node/src/db.rs`: append the 037 entry to `SCHEMA` exactly like the 036 entry (db.rs:189-190).
- `crates/cairn-node/tests/twin_registry.rs`: count 18→19 (line ~98-103) and add to the `expected` vec: `("erasure.shred.asserted", Some("cairn_check_erasure_shred"), Some("erasure.shred requires a non-empty authored twin (the tombstone must be legible — ADR-0052)"))` in sort position.
- `db/tests/034_twin_registry_test.sql`: `IF n <> 18` → `IF n <> 19`, update the arithmetic comment (`19 = 18 + 1 (ADR-0052 erasure.shred)`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test born_sealed_schema --test twin_registry`
Expected: PASS (all).
Also: `psql "$CAIRN_TEST_PG" -f db/tests/034_twin_registry_test.sql` → `OK` notices.

- [ ] **Step 6: Full workspace check + commit**

```bash
CAIRN_TEST_PG=… cargo test --workspace
git add db/037_born_sealed.sql crates/cairn-node/src/db.rs crates/cairn-node/tests db/tests/034_twin_registry_test.sql
git commit -m "feat(#189,#92): db/037 — custody plane (event_dek/event_clear/shred log) + erasure.shred.asserted (twin mirrors 18→19)"
```

---

### Task 6: Projection triggers read through cairn_clear_payload

**Files:**
- Modify: `db/031_medication.sql` (`medication_statement_apply` :160, `medication_cessation_apply`-equivalent :233), `db/032_medication_dose.sql` (:154, :179, :291), `db/033_medication_reconciliation.sql` (:177), `db/034_medication_attestation.sql` (:179), `db/035_medication_dose_effective_correction.sql` (:151)
- Test: extend `crates/cairn-node/tests/born_sealed_schema.rs`

**Interfaces:**
- Consumes: `cairn_clear_payload(event_log)` (Task 5).
- Produces: every medication projection trigger fn now begins with the clear-payload idiom; all 700 existing tests stay green (unsealed rows resolve to `NEW.body` unchanged).

- [ ] **Step 1: Write the failing test**

Append to `born_sealed_schema.rs` (uses raw SQL row insertion? No — event_log has no direct INSERT; drive it through `cairn_clear_payload` directly):

```rust
#[tokio::test]
async fn clear_payload_resolves_unsealed_to_body_and_sealed_to_shadow() {
    let Some(conn) = cs() else { return };
    let base = db::connect(&conn).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&conn).await.unwrap();
    // Composite-type call against synthesized rows: unsealed row → body.
    let body: serde_json::Value = c.query_one(
        "SELECT cairn_clear_payload(e)::text FROM (
            SELECT (NULL::event_log) #= hstore(ARRAY['sealed','body'], ARRAY['false','{\"k\":1}']) AS e
         ) s", &[],
    ).await.map(|r| serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(0)).unwrap())
     .unwrap_or_else(|_| serde_json::json!(null));
    // If hstore isn't installed, fall back to a real end-to-end check in Task 7's
    // sealed-submit test instead — then this test only pins the SEALED-no-custody
    // NULL path via a manufactured event_clear-less lookup:
    let _ = body;
    let is_null: bool = c.query_one(
        "SELECT cairn_clear_payload(ROW(gen_random_uuid(), gen_random_uuid(), 'clinical.medication.asserted',
                'clinical.medication/1', 0, 0, 'n', NULL, '\\x00'::bytea, '\\x00'::bytea,
                '{}'::jsonb, '[]'::jsonb, 'k', 'stub', TRUE, NULL, '[]'::jsonb,
                clock_timestamp(), NULL, NULL, NULL, NULL)::event_log) IS NULL", &[],
    ).await.unwrap().get(0);
    assert!(is_null, "sealed row with no event_clear shadow must resolve NULL");
}
```

**NOTE to implementer:** the `ROW(...)::event_log` literal must list event_log's columns in exact DDL order (db/001 + its ALTER ADD COLUMNs + db/036's `seq` — run `\d event_log` and transcribe; `GENERATED ALWAYS` seq may refuse a cast — if the composite cast fights back, DELETE this synthetic test and rely on Task 7's end-to-end sealed-submit test, which covers both paths for real; do not burn time on synthetic row literals).

- [ ] **Step 2: Edit each trigger function**

In each listed trigger fn, replace the declaration line

```sql
    p jsonb := NEW.body;
```

with

```sql
    -- ADR-0052: sealed rows carry ciphertext in body; the clear payload lives
    -- in event_clear (populated by the door BEFORE this row, same txn). NULL =
    -- sealed without custody here: nothing to project — honest degradation.
    p jsonb := cairn_clear_payload(NEW);
```

and add immediately after the fn's `BEGIN`:

```sql
    IF p IS NULL THEN RETURN NULL; END IF;
```

Eight functions total (031×2, 032×3, 033×1, 034×1, 035×1). `db/002_projection.sql`'s `event_log_project` is NOT touched (walking-skeleton `patient.*` types are not `clinical.*` and stay plaintext). If db/033's reconciliation fn declares `p` differently (`p jsonb := NEW.body;` inside a multi-var DECLARE), apply the same substitution to just that line.

- [ ] **Step 3: Run the full medication regression**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication --test medication_dose --test medication_reconciliation --test medication_attestation` (use the actual test-file names — `ls crates/cairn-node/tests | grep medication`)
Expected: ALL PASS — unsealed events project exactly as before (`cairn_clear_payload` returns `NEW.body`).

- [ ] **Step 4: Commit**

```bash
git add db/03*.sql crates/cairn-node/tests/born_sealed_schema.rs
git commit -m "feat(#189): medication projections read via cairn_clear_payload — sealed-aware, unsealed unchanged"
```

---

### Task 7: Submit door sealed arm (db/005)

**Files:**
- Modify: `db/005_submit.sql` (signature + sealed arm; GRANT line :575)
- Test: `crates/cairn-node/tests/seal_submit.rs` (new)

**Interfaces:**
- Consumes: `cairn_unseal_body`, `cairn_wrap_dek` (Task 4), Task 5 tables, `cairn_event::seal` (test side).
- Produces: `submit_event(p_signed BYTEA, p_attestation BYTEA DEFAULT NULL, p_attester_key BYTEA DEFAULT NULL, p_dek BYTEA DEFAULT NULL) RETURNS UUID`. Behavior contract for later tasks: sealed + DEK → full strict checks on the clear body, custody + shadow stored, projections fire; sealed without DEK at THIS door → refusal; unsealed `clinical.*` at THIS door → refusal; already-shredded target → row admitted, custody/shadow withheld.

- [ ] **Step 1: Write the failing tests**

`crates/cairn-node/tests/seal_submit.rs` — boilerplate from `medication.rs:1-45` (`cs`, `db_msg`, `setup_node` enrolling a device actor). Test bodies build a medication assert `EventBody` exactly as `crates/cairn-node/src/medication/assert.rs:39-73` does (copy the field construction), then seal:

```rust
//! ADR-0052 strict-door sealed arm. DB-gated on $CAIRN_TEST_PG.

use cairn_event::seal::{seal_event_payload, seal_stub_twin, derive_unwrap_secret, unwrap_public};
use cairn_event::{sign, EventBody, Hlc};

/// Build a CLEAR medication-assert EventBody (mirror of medication/assert.rs
/// build_assert_body), then seal it: payload → container, twin → stub.
/// Returns (sealed body ready to sign, dek).
fn sealed_assert_body(node_kid: &str, patient: uuid::Uuid, hlc: (i64, i32, &str))
    -> (EventBody, zeroize::Zeroizing<[u8; 32]>)
{
    let event_id = uuid::Uuid::now_v7().to_string();
    let medication_id = uuid::Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": medication_id,
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let twin = format!("amoxicillin — asserted for {patient}");
    let (container, dek) = seal_event_payload(&payload, &twin, &event_id).unwrap();
    let body = EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc: Hlc { wall: hlc.0, counter: hlc.1, node_origin: hlc.2.into() },
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
    };
    (body, dek)
}

#[tokio::test]
async fn sealed_submit_with_dek_projects_and_stores_custody() { /* :
    setup; register unwrap key:
        let secret = derive_unwrap_secret(&sk.to_bytes());
        c.execute("SELECT cairn_register_unwrap_key($1)", &[&unwrap_public(&secret).as_slice()]).await?;
    build sealed body; sign; 
        c.execute("SELECT submit_event($1, NULL, NULL, $2)", &[&signed.signed_bytes, &dek.as_slice()]).await?;
    then assert:
      - event_log row: sealed = true; body = the container (body->>'sealed' = 'true');
        plaintext_twin = the stub (contains 'twin under seal'); body::text does NOT contain 'amoxicillin'.
      - event_clear row exists: body->'substance'->>'term' = 'amoxicillin'; twin contains 'amoxicillin'.
      - event_dek row exists (104-byte dek_wrapped) and unwrap_dek(dek_wrapped, secret) == dek.
      - medication_statement row exists with term 'amoxicillin' (projection fired through the shadow).
*/ }

#[tokio::test]
async fn sealed_submit_without_dek_is_refused_legibly() { /* same body, call
    submit_event($1) 1-arg → expect error containing 'requires its DEK' */ }

#[tokio::test]
async fn unsealed_clinical_body_is_refused_at_the_strict_door() { /* build the
    SAME body but payload = clear payload, plaintext_twin = Some(twin) (no seal);
    submit_event($1) → expect error containing 'born-sealed' (ADR-0052 floor) */ }

#[tokio::test]
async fn wrong_dek_is_refused() { /* sealed body signed, but pass a derived
    wrong 32-byte dek → expect error containing 'failed to open' */ }
```

Write all four in full (the comments above are the assertions to express in real code; follow medication.rs patterns for enrollment and error text via `db_msg`).

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test seal_submit`
Expected: FAIL — `submit_event` has no 4th parameter / no sealed handling.

- [ ] **Step 3: Edit db/005_submit.sql**

1. **Signature** (:352-360): add `p_dek BYTEA DEFAULT NULL` as 4th arg. Immediately BEFORE the `CREATE OR REPLACE FUNCTION submit_event` add:

```sql
-- ADR-0052: the door gained p_dek. A CREATE OR REPLACE with a different arg
-- list would OVERLOAD (3-arg + 4-arg → ambiguous 1-arg calls), so drop the old
-- signature first. Idempotent across replays.
DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);
```

2. **DECLARE block**: add `v_sealed boolean := false; b_clear jsonb; v_inner jsonb; v_pub bytea; v_twin_stub text;`

3. **Sealed detection + open + posture floor** — insert between step 6 (advisory provenance, :520-524) and step 7 (twin, :526):

```sql
    -- 6b. ADR-0052 born-sealed arm. A clinical body arrives EITHER as the sealed
    --     container (payload.sealed = true) — the shipped default — or as legacy
    --     plaintext, which the STRICT door refuses: an unsealed clinical body is
    --     permanently un-shreddable, and this floor is what makes the posture
    --     unbypassable (principle 12). The apply door stays lenient (set-union).
    v_sealed := COALESCE((b -> 'payload' ->> 'sealed')::boolean, false);
    b_clear  := b;
    IF v_sealed THEN
        IF p_dek IS NULL THEN
            RAISE EXCEPTION 'submit_event: sealed event requires its DEK at the strict door (ADR-0052)';
        END IF;
        v_inner := cairn_unseal_body(b -> 'payload', p_dek, v_event_id::text);
        IF v_inner IS NULL THEN
            RAISE EXCEPTION 'submit_event: sealed body failed to open with the presented DEK (wrong key, tampered container, or event-id mismatch) — refused (ADR-0052)';
        END IF;
        v_twin_stub := b ->> 'plaintext_twin';
        IF COALESCE(v_twin_stub, '') = '' THEN
            RAISE EXCEPTION 'submit_event: sealed event must carry a signed plaintext twin STUB (principle 11 — the row must stay self-describing) (ADR-0052)';
        END IF;
        -- The floor checks below run on the CLEAR view; the log stores ciphertext.
        b_clear := jsonb_set(jsonb_set(b, '{payload}', v_inner -> 'payload'),
                             '{plaintext_twin}', v_inner -> 'plaintext_twin');
    ELSIF v_type LIKE 'clinical.%' THEN
        RAISE EXCEPTION 'submit_event: % is a clinical body and must be born-sealed — plaintext clinical submissions are refused at the strict door (ADR-0052; wipe pre-ADR-0052 dev rigs, never sync them through)', v_type;
    END IF;
```

4. **Twin dispatch** (:529): change `v_twin := cairn_event_twin(v_type, b);` → `v_twin := cairn_event_twin(v_type, b_clear);`

5. **Custody + shadow, BEFORE the event_log INSERT** (immediately after the twin dispatch):

```sql
    -- 6c. Custody + operational clear view — BEFORE the log INSERT so the AFTER
    --     INSERT projection triggers can already read the shadow (same txn).
    --     An already-shredded target gets NEITHER: set-union may re-deliver the
    --     row forever, but custody never resurrects (arrival-order independence).
    IF v_sealed AND NOT EXISTS (SELECT 1 FROM erasure_shred_log WHERE target_event_id = v_event_id) THEN
        SELECT unwrap_pub INTO v_pub FROM node_unwrap_key;
        IF v_pub IS NULL THEN
            RAISE EXCEPTION 'submit_event: node unwrap key not registered — the authoring daemon must call cairn_register_unwrap_key first (ADR-0052)';
        END IF;
        INSERT INTO event_dek (event_id, dek_wrapped)
        VALUES (v_event_id, cairn_wrap_dek(p_dek, v_pub))
        ON CONFLICT (event_id) DO NOTHING;
        INSERT INTO event_clear (event_id, body, twin)
        VALUES (v_event_id, b_clear -> 'payload', v_twin)
        ON CONFLICT (event_id) DO NOTHING;
    END IF;
```

6. **The INSERT** (:531-543): `body` value stays `b -> 'payload'` (the honest derived view — the ciphertext container for sealed rows); `plaintext_twin` value becomes `CASE WHEN v_sealed THEN v_twin_stub ELSE v_twin END`; add `sealed` to the column list with value `v_sealed`.

7. **Shred arm** — after `cairn_learn_attachment_refs` (:557):

```sql
    -- 6d. The erasure plane: an admitted shred tombstone EXECUTES here (ADR-0052).
    --     Strict door: the target must exist locally (shredding the unknown is a
    --     user error at authoring time; the APPLY door is lenient — a shred may
    --     arrive before its target on the wire).
    IF v_type = 'erasure.shred.asserted' THEN
        IF NOT EXISTS (SELECT 1 FROM event_log
                       WHERE event_id = (b_clear -> 'payload' ->> 'target_event_id')::uuid) THEN
            RAISE EXCEPTION 'submit_event: erasure.shred targets unknown event % — nothing to shred here', b_clear -> 'payload' ->> 'target_event_id';
        END IF;
        PERFORM cairn_execute_shred(
            (b_clear -> 'payload' ->> 'target_event_id')::uuid,
            v_event_id, b_clear -> 'payload' ->> 'basis');
    END IF;
```

8. **GRANT** (:575): update to `GRANT EXECUTE ON FUNCTION submit_event(bytea, bytea, bytea, bytea) TO cairn_agent;`

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test seal_submit`
Expected: 4 PASS.

- [ ] **Step 5: Run the whole cairn-node suite — expect medication authoring breakage**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node`
Expected: the medication tests that author via `assert_medication`/hand-crafted plaintext bodies now FAIL against the born-sealed floor. **That is the RED state for Task 10** — do NOT patch tests to bypass the floor. Record the failing list in the Task 10 notes, then move on (Tasks 8–9 are door-side and testable independently). If unrelated suites fail, stop and fix before committing.

Commit only db/005 + the new test (the suite-wide red is documented, deliberate, and resolved by Task 10 — note it in the commit message):

```bash
git add db/005_submit.sql crates/cairn-node/tests/seal_submit.rs
git commit -m "feat(#189): submit door sealed arm — born-sealed floor, in-DB unseal, custody+shadow, shred execution (medication authoring converts in the next commits)"
```

---

### Task 8: Apply door custody arm (db/020)

**Files:**
- Modify: `db/020_apply_remote_event.sql` (signature + sealed/lenient arm; GRANT :292-296)
- Test: `crates/cairn-node/tests/seal_apply.rs` (new)

**Interfaces:**
- Consumes: everything Task 7 consumes.
- Produces: `apply_remote_event(p_signed BYTEA, p_attestation BYTEA DEFAULT NULL, p_attester_key BYTEA DEFAULT NULL, p_dek BYTEA DEFAULT NULL) RETURNS UUID`. Contract: sealed + DEK → clear checks + custody + shadow + projections (as submit); sealed without DEK → ADMIT structural-only (stub twin stored, no custody/shadow/projection — never reject); plaintext clinical → ADMIT leniently (foreign legacy data, set-union); shredded target → admit row, withhold custody.

- [ ] **Step 1: Write the failing tests**

`crates/cairn-node/tests/seal_apply.rs`, reusing `sealed_assert_body` (copy the helper — integration tests can't share; or add `mod common;` if one exists):

```rust
#[tokio::test]
async fn sealed_apply_with_dek_projects_like_submit() { /* register unwrap key;
    apply_remote_event($1, NULL, NULL, $2) with the DEK → event_clear + event_dek
    + medication_statement all present; sealed=true; body is the container */ }

#[tokio::test]
async fn sealed_apply_without_dek_admits_structurally_never_rejects() { /*
    apply_remote_event($1) one-arg on a sealed body → returns the uuid (ADMITTED);
    event_log row exists with sealed=true, plaintext_twin = the stub;
    NO event_clear row, NO event_dek row, NO medication_statement row.
    This is the ADR-0051 lenient-apply precedent applied to the seal. */ }

#[tokio::test]
async fn plaintext_clinical_apply_is_admitted_leniently() { /* the unsealed
    clinical body that Task 7 proved REFUSED at submit → apply_remote_event
    ADMITS it and projects it (set-union losslessness for foreign/legacy data) */ }

#[tokio::test]
async fn custody_is_never_granted_to_a_shredded_event() { /* submit a sealed
    event on this node via the full path; author+apply an erasure.shred for it
    (build via a helper `shred_body(target, basis, …)` mirroring
    sealed_assert_body but type erasure.shred.asserted, PLAINTEXT payload
    {"target_event_id":…, "basis":"retention ceiling"} and an authored twin);
    then RE-apply the original sealed event WITH its DEK (set-union re-delivery)
    → row conflict-ignored, event_dek/event_clear STAY EMPTY,
    medication_statement STAYS EMPTY (arrival-order independence half 1). */ }
```

Note for the shred fixture: `erasure.shred.asserted` is NOT `clinical.%`, so its plaintext payload passes both doors; its twin is authored ("shredded medication assertion <uuid> — basis: retention ceiling"). The strict attestation gate only fires if a contributor claims responsibility — for door tests author it with the plain `recorded` role (no responsibility object), matching how other door tests author events. (The human-vouched shred ceremony is the CLI's job — Task 11.)

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test seal_apply`
Expected: FAIL (no 4th param, no sealed handling).

- [ ] **Step 3: Edit db/020_apply_remote_event.sql**

Mirror Task 7's edits with the lenient differences:

1. `DROP FUNCTION IF EXISTS apply_remote_event(bytea, bytea, bytea);` before the CREATE; add `p_dek BYTEA DEFAULT NULL`.
2. Same DECLARE additions.
3. Sealed arm (before the twin dispatch at :217):

```sql
    -- ADR-0052 lenient arm: a sealed event NEVER rejects here. With the DEK the
    -- full floor runs on the clear view; without it (not a custody holder, or a
    -- pre-seal peer) the row is admitted on structural checks only — set-union
    -- losslessness. Plaintext clinical bodies are likewise ADMITTED (foreign /
    -- pre-ADR-0052 data); only the STRICT door enforces born-sealed.
    v_sealed := COALESCE((b -> 'payload' ->> 'sealed')::boolean, false);
    b_clear  := b;
    IF v_sealed AND p_dek IS NOT NULL THEN
        v_inner := cairn_unseal_body(b -> 'payload', p_dek, v_event_id::text);
        IF v_inner IS NULL THEN
            -- A presented-but-wrong DEK is a transport defect, not a reason to
            -- lose the event: admit structurally, custody stays withheld.
            RAISE WARNING 'apply_remote_event: sidecar DEK failed to open sealed body % — admitting without custody', v_event_id;
        ELSE
            b_clear := jsonb_set(jsonb_set(b, '{payload}', v_inner -> 'payload'),
                                 '{plaintext_twin}', v_inner -> 'plaintext_twin');
        END IF;
    END IF;
    v_twin_stub := b ->> 'plaintext_twin';
```

4. Twin dispatch: replace `v_twin := cairn_event_twin(v_type, b);` with

```sql
    IF v_sealed AND (v_inner IS NULL) THEN
        -- No custody: the structural check fn cannot read the body. Store the
        -- signed stub; degrade to skeleton if the author omitted one.
        v_twin := COALESCE(NULLIF(v_twin_stub, ''), cairn_twin_skeleton(v_type, b));
    ELSE
        v_twin := cairn_event_twin(v_type, b_clear);
    END IF;
```

5. Custody + shadow before the INSERT — same block as Task 7 step 3.5 but conditioned `IF v_sealed AND v_inner IS NOT NULL AND NOT EXISTS (… erasure_shred_log …)` and with the unwrap-key-missing case downgraded to `RAISE WARNING` + skip (a pulling node that never registered still must not lose the event).
6. INSERT: `sealed` column + `plaintext_twin = CASE WHEN v_sealed THEN v_twin_stub ELSE v_twin END` (same as submit; if `v_twin_stub` is NULL for a sealed foreign event, fall back to `v_twin`).
7. Shred arm after the insert — lenient: NO target-existence requirement:

```sql
    IF v_type = 'erasure.shred.asserted' THEN
        -- Lenient: the shred may precede its target on the wire. Recording it
        -- in erasure_shred_log is what makes the LATER-arriving target refuse
        -- custody (arrival-order independence half 2).
        PERFORM cairn_execute_shred(
            (b -> 'payload' ->> 'target_event_id')::uuid,
            v_event_id, COALESCE(b -> 'payload' ->> 'basis', '(unrecorded)'));
    END IF;
```

8. GRANT → 4-arg signature, `TO cairn_node`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test seal_apply --test apply_remote_event --test seal_submit`
Expected: PASS (the pre-existing apply_remote_event suite must stay green — it applies non-clinical and legacy events).

- [ ] **Step 5: Commit**

```bash
git add db/020_apply_remote_event.sql crates/cairn-node/tests/seal_apply.rs
git commit -m "feat(#189,#92): apply door custody arm — lenient sealed admission, sidecar DEK, shred-before-target independence"
```

---

### Task 9: Shred order-independence + shred-arrives-after test (both directions pinned)

**Files:**
- Test: extend `crates/cairn-node/tests/seal_apply.rs`

**Interfaces:**
- Consumes: Tasks 7–8 doors.
- Produces: the pinned contract Task 13's E2E relies on.

- [ ] **Step 1: Write the failing-or-passing pins**

Task 8 pinned shred-before-event. Add the other direction plus the audit invariants:

```rust
#[tokio::test]
async fn shred_after_event_scrubs_everything_but_the_log_row() { /*
    full sealed submit (custody + shadow + projection present);
    submit the erasure.shred (strict door, target exists);
    then assert: event_dek empty for target; event_clear empty; medication_statement
    empty (content_address-precise); erasure_shred_log has the row with basis;
    event_log still has BOTH rows (target + tombstone); target row's signed_bytes
    still verify: SELECT cairn_verify(signed_bytes) FROM event_log WHERE event_id=$1
    → true (signature-over-ciphertext survives the shred — ADR-0005). */ }

#[tokio::test]
async fn shred_is_idempotent_under_replay() { /* apply the SAME shred event
    twice (set-union re-delivery): second apply returns the uuid, no error,
    erasure_shred_log still one row. */ }

#[tokio::test]
async fn schema_reload_rebuilds_the_shred_log() { /* after the shred:
    DELETE FROM erasure_shred_log (simulating a projection-state wipe);
    db::connect_and_load_schema again (the restore/replay path);
    erasure_shred_log repopulated from the event_log tombstone —
    "restore replays the shred log" (§3.8). */ }
```

- [ ] **Step 2: Run, implement any door gap, pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test seal_apply`
Expected: likely PASS from Tasks 7–8; if a pin fails, the door arm has a real gap — fix in db/005/db/020/db/037, never in the test.

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/seal_apply.rs
git commit -m "test(#92): pin shred order-independence, replay idempotence, and log-rebuild-on-reload"
```

---

### Task 10: cairn-node seal-at-write — all medication verbs through one sealed submit helper

**Files:**
- Create: `crates/cairn-node/src/medication/sealed_submit.rs`
- Modify: `crates/cairn-node/src/medication/{mod.rs, assert.rs, cessation.rs, dose.rs, attestation.rs, reconciliation.rs}` — every `sign` + `SELECT submit_event(...)` site
- Modify: `crates/cairn-node/src/keystore.rs` (unwrap-secret accessor)
- Test: the EXISTING medication suites (currently red from Task 7) + extend `seal_submit.rs`

**Interfaces:**
- Consumes: `seal_event_payload`, `seal_stub_twin`, `derive_unwrap_secret`, `unwrap_public` (Tasks 2–3); 4-arg doors (Task 7).
- Produces:
  - `keystore.rs`: `pub fn unwrap_secret(sk: &SigningKey) -> Zeroizing<[u8; 32]>` (thin wrapper over `derive_unwrap_secret(&sk.to_bytes())`)
  - `medication/sealed_submit.rs`:
    - `pub async fn ensure_unwrap_key(client: &tokio_postgres::Client, sk: &SigningKey) -> anyhow::Result<()>` — derives the public half, calls `SELECT cairn_register_unwrap_key($1)` (idempotent)
    - `pub async fn seal_sign_submit(client: &mut tokio_postgres::Client, sk: &SigningKey, mut body: EventBody, attest: Option<&AttestParams<'_>>) -> anyhow::Result<uuid::Uuid>` — takes the CLEAR body (payload + Some(clear twin)), seals payload+twin into the container, sets the stub, signs, registers the unwrap key, submits `SELECT submit_event($1, $2, $3, $4)` (attested variant runs in the same txn pattern as `assert.rs:107-115` today)

- [ ] **Step 1: The RED state already exists**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication`
Expected: FAIL with the born-sealed refusal from Task 7 — this is the driving red.

- [ ] **Step 2: Implement `sealed_submit.rs`**

```rust
//! ADR-0052 seal-at-write: the ONE path every clinical verb submits through.
//! Takes the verb's CLEAR EventBody (pure builders unchanged — they stay
//! testable in cairn-event), seals payload+twin under a fresh per-event DEK,
//! signs the SEALED form (seal-then-sign: the signature covers ciphertext and
//! survives a shred), and hands the DEK to the strict door for floor checks,
//! custody wrap, and the operational clear view.

use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, SigningKey};

use crate::keystore;

pub async fn ensure_unwrap_key(
    client: &tokio_postgres::Client,
    sk: &SigningKey,
) -> anyhow::Result<()> {
    let secret = keystore::unwrap_secret(sk);
    let public = cairn_event::seal::unwrap_public(&secret);
    client
        .execute("SELECT cairn_register_unwrap_key($1)", &[&public.as_slice()])
        .await?;
    Ok(())
}

pub async fn seal_sign_submit(
    client: &mut tokio_postgres::Client,
    sk: &SigningKey,
    mut body: EventBody,
    attest: Option<&super::AttestParams<'_>>,
) -> anyhow::Result<uuid::Uuid> {
    let clear_twin = body
        .plaintext_twin
        .take()
        .ok_or_else(|| anyhow::anyhow!("seal_sign_submit: clear body must carry its twin"))?;
    let (container, dek) = seal_event_payload(&body.payload, &clear_twin, &body.event_id)?;
    body.payload = container;
    body.plaintext_twin = Some(seal_stub_twin(&body.event_type));
    let signed = sign(&body, sk)?;
    ensure_unwrap_key(client, sk).await?;
    // Mirror the existing attested-vs-plain split in medication/assert.rs
    // (single txn for the attested pair), with the DEK as the 4th door arg:
    //   SELECT submit_event($1, NULL, NULL, $2)                 -- plain
    //   SELECT submit_event($1, $2, $3, $4) + attest_thread_in_tx -- attested
    ...
}
```

(Implementer: transplant the txn/attest logic from `assert.rs:100-118` into this helper once, then delete it from the verb files. `AttestParams` lives in the medication module — check `medication/mod.rs` re-exports and keep them compiling.)

`keystore.rs` addition:

```rust
/// ADR-0052: the node's X25519 DEK-unwrap secret, HKDF-derived from the SAME
/// Ed25519 seed the keystore already seals and escrows (ADR-0026) — one master
/// secret, two independent keys, no second recovery ceremony. Domain-separated
/// by the HKDF info tag in cairn-event::seal.
pub fn unwrap_secret(sk: &SigningKey) -> zeroize::Zeroizing<[u8; 32]> {
    cairn_event::seal::derive_unwrap_secret(&sk.to_bytes())
}
```

- [ ] **Step 3: Convert every verb**

In each of `assert.rs`, `cessation.rs`, `dose.rs` (two verbs), `attestation.rs`, `reconciliation.rs` (two verbs): the function keeps building its clear `EventBody` exactly as today (pure builders + `plaintext_twin: Some(render_*_twin(..))`), then replaces its `sign` + `submit_event` block with `sealed_submit::seal_sign_submit(client, node_sk, body, attest).await`. **Exception:** `attestation.rs` — check whether the attestation event type is `clinical.%`; it is (`clinical.medication-attestation.*` or similar — read the file). All `clinical.%` types must seal; the same helper handles them.

- [ ] **Step 4: Fix remaining red tests that hand-craft plaintext clinical bodies**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node`
For every remaining failure that hand-crafts a plaintext clinical `EventBody` and calls `submit_event` directly: convert the fixture with the same seal steps (`seal_event_payload` → container → stub → sign → 4-arg submit). Where several test files need it, copy the small `sealed_assert_body`-style helper — do NOT weaken any assertion. Tests that intentionally pin refusals must keep pinning them (some now pin the NEW born-sealed refusal text — update expected substrings only where the refusal legitimately changed).

Expected end state: `cargo test -p cairn-node` fully green.

- [ ] **Step 5: fmt/clippy + commit**

```bash
cargo fmt && cargo clippy -p cairn-node --all-targets -- -D warnings
git add crates/cairn-node
git commit -m "feat(#189): seal-at-write — every medication verb submits sealed via one helper; unwrap key auto-registered"
```

---

### Task 11: cairn-node shred CLI

**Files:**
- Create: `crates/cairn-node/src/shred.rs`
- Modify: `crates/cairn-node/src/main.rs` (new `Cmd::Shred` variant + handler + help text), `crates/cairn-node/src/lib.rs` (`pub mod shred;`)
- Test: `crates/cairn-node/tests/shred_cli.rs` (new; drives `shred::shred_event` directly like other verb tests drive their module fns)

**Interfaces:**
- Consumes: doors (Task 7), `db::next_hlc`, `identity::load_local`, the attestation machinery (`resolve_attester`/`attest_params` pattern from the medication handler at main.rs:1659-1702).
- Produces: `pub async fn shred_event(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str, node_origin: &str, target: uuid::Uuid, basis: &str, attest: Option<&medication::AttestParams<'_>>) -> anyhow::Result<uuid::Uuid>`; CLI `cairn-node shred --conn … --key … --event <uuid> --basis <text> [--attest <human-kid>]`.

- [ ] **Step 1: Write the failing test**

```rust
//! ADR-0052 rung-3 shred ceremony. DB-gated on $CAIRN_TEST_PG.

#[tokio::test]
async fn shred_event_appends_tombstone_and_scrubs() { /*
    setup; seal-submit a medication assert via medication::assert_medication;
    confirm medication_statement + event_dek rows exist;
    shred::shred_event(&mut c, &sk, &kid, &origin, event_id, "retention ceiling", None).await?;
    assert: erasure_shred_log row (basis = 'retention ceiling');
    event_dek/event_clear/medication_statement scrubbed;
    the tombstone event_log row's plaintext_twin CONTAINS the target uuid and the
    basis (the legible audit record); patient_id on the tombstone = target's patient. */ }

#[tokio::test]
async fn shred_refuses_an_unknown_target_legibly() { /* random uuid →
    error contains 'nothing to shred' */ }
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p cairn-node --test shred_cli` → compile FAIL.

- [ ] **Step 3: Implement `shred.rs`**

Model on `medication/assert.rs` end-to-end shape. The body:

```rust
//! The rung-3 audited crypto-shred ceremony (ADR-0005 / ADR-0052).
//! The tombstone is a PLAINTEXT clinical-plane event by design: "existed →
//! destroyed, basis Z" must outlive every key. The heavy lifting (custody
//! destruction + derived-plaintext scrub) happens inside the door's shred arm,
//! so a raw-SQL client cannot shred without also leaving the tombstone.

pub async fn shred_event(/* signature above */) -> anyhow::Result<Uuid> {
    // 1. Read the target's patient_id (the tombstone lands in the same chart).
    //    Refuse legibly if the target is unknown (the strict door double-checks).
    // 2. Build the EventBody: event_type "erasure.shred.asserted",
    //    schema_version "erasure.shred/1", payload {"target_event_id", "basis"},
    //    plaintext_twin Some(format!("shredded event {target} — basis: {basis}")),
    //    contributors [{actor_id: node_kid, role: "recorded"}] (or the attested
    //    responsibility shape when attest is Some — mirror assert.rs).
    // 3. sign() — PLAINTEXT submit: SELECT submit_event($1) / attested 3-arg txn.
    //    (erasure.* is exempt from the born-sealed floor; no DEK.)
}
```

Write it in full following `assert.rs:82-118`'s structure. CLI wiring in `main.rs`: new `Cmd::Shred { event: Uuid, basis: String, attest: Option<String> }` + handler mirroring the medication-assert handler's key-loading and attester resolution; add one help line under the medication commands (main.rs help text ~:2272-2294).

- [ ] **Step 4: Run tests to verify they pass** — `cargo test -p cairn-node --test shred_cli` → PASS.

- [ ] **Step 5: fmt/clippy + commit**

```bash
git add crates/cairn-node
git commit -m "feat(#189): cairn-node shred — the rung-3 audited crypto-shred ceremony CLI"
```

---

### Task 12: cairn-sync custody sidecar on the clinical wire

**Files:**
- Modify: `crates/cairn-sync/src/main.rs`:
  - `Request::EventsAfterSeq` (:249) gains `#[serde(default)] unwrap_cert: Option<String>`
  - `EventsResponse` (:258-291) gains `#[serde(default)] wrapped_deks: Vec<Option<String>>`
  - serve arms (:2157-2183, :2184-2211): custody join + per-requester re-wrap
  - `do_pull` (:1218) + `apply_signed` (:470): sidecar decode → daemon-side unwrap → 4-arg door
- Test: unit tests beside `events_response_seqs_field_is_additive` (:2706-2727) + the E2E in Task 13

**Interfaces:**
- Consumes: `sign_unwrap_key_cert`/`verify_unwrap_key_cert`, `derive_unwrap_secret`/`unwrap_public`/`unwrap_dek`/`wrap_dek_for` (Task 3); `event_dek`/`erasure_shred_log` (Task 5); 4-arg `apply_remote_event` (Task 8); `load_or_create_key` (main.rs:404) for the seed.
- Produces: the wire contract Task 13 exercises. Additive: an old peer ignores/omits both fields and everything still syncs (events flow without custody — sealed rows admit structurally).

- [ ] **Step 1: Write the failing unit tests** (pure serde, no DB — beside the existing additive-decode test):

```rust
    #[test]
    fn events_response_wrapped_deks_field_is_additive() {
        // Old responder: no field → empty vec (puller treats as no custody).
        let old = r#"{"events":[],"attestations":[],"attester_keys":[],"seqs":[]}"#;
        let r: EventsResponse = serde_json::from_str(old).unwrap();
        assert!(r.wrapped_deks.is_empty());
    }

    #[test]
    fn events_after_seq_unwrap_cert_is_additive() {
        let old = r#"{"EventsAfterSeq":{"after_seq":0}}"#;
        let r: Request = serde_json::from_str(old).unwrap();
        match r { Request::EventsAfterSeq { unwrap_cert, .. } => assert!(unwrap_cert.is_none()),
                  _ => panic!("wrong variant") }
    }

    #[test]
    fn unwrap_key_cert_round_trip_binds_kid() {
        let sk = cairn_event::generate_key();
        let secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
        let xpub = cairn_event::seal::unwrap_public(&secret);
        let cert = cairn_event::sign_unwrap_key_cert(&sk, &xpub).unwrap();
        let (kid, got) = cairn_event::verify_unwrap_key_cert(&cert).unwrap();
        assert_eq!(kid, hex::encode(sk.verifying_key().to_bytes()));
        assert_eq!(got, xpub);
    }
```

(Match the existing Request serde representation — check how `EventsAfterSeq` actually serializes (externally tagged?) by reading the existing wire tests, and write the JSON literal accordingly.)

- [ ] **Step 2: Run to verify failure** — `cargo test -p cairn-sync` → compile FAIL.

- [ ] **Step 3: Implement**

**Puller side (`do_pull`):**
1. Load the seed once (the existing `--key` path already feeds `load_or_create_key`); derive `unwrap_secret`/`unwrap_public`; build `unwrap_cert = hex(sign_unwrap_key_cert(&sk, &public))`; include in `Request::EventsAfterSeq`.
2. Decode `resp.wrapped_deks.get(i)` like attestations (:1367-1385); hex-decode; `unwrap_dek(&wrapped, &unwrap_secret)` — on failure log a warning and pass None (the door admits structurally; never drop the event).
3. Thread `Option<Zeroizing<[u8;32]>>` into `apply_signed` → `SELECT apply_remote_event($1, $2, $3, $4)` with `dek.as_ref().map(|d| d.as_slice())`.

**Server side (both serve arms):**
1. Extend the SQL: `LEFT JOIN event_dek d ON d.event_id = e.event_id LEFT JOIN erasure_shred_log s ON s.target_event_id = e.event_id` selecting `CASE WHEN s.target_event_id IS NULL THEN encode(d.dek_wrapped, 'hex') END AS dek_hex` (shredded events NEVER ship custody — the wire-level half of the shred guarantee).
2. If the request carried `unwrap_cert`: `verify_unwrap_key_cert` → `(kid, requester_pub)`. Custody follows admission trust for erasable-tier rows: re-wrap each served DEK — `unwrap_dek(local_wrap, own_secret)` then `wrap_dek_for(&dek, &requester_pub)` — and hex it into `wrapped_deks[i]`. On cert absence/invalid: serve events with `wrapped_deks` all-None (never refuse the pull). Add a `// TODO(#follow-up)` comment + file the issue in Task 14: pin the cert kid against the node-plane trust set (skeleton verifies signature + self-consistency only).
3. Old-peer path (`EventsAfter` HLC arm): leave untouched (no sidecar — legacy).

- [ ] **Step 4: Run tests to verify they pass** — `cargo test -p cairn-sync` → PASS (unit level).

- [ ] **Step 5: fmt/clippy + commit**

```bash
git add crates/cairn-sync
git commit -m "feat(#189): clinical wire custody sidecar — unwrap-cert request, per-peer DEK re-wrap, shred-aware exclusion"
```

---

### Task 13: E2E through the real binaries + seal bench

**Files:**
- Modify: `crates/cairn-sync/tests/clinical_pull.rs` (new scenario) — follow its existing two-DB harness (`CAIRN_TEST_PG`/`CAIRN_TEST_PG2`, real `cairn-sync` binary spawn)
- Modify: `crates/cairn-sync/src/main.rs` (a `bench-seal` subcommand next to `bench-insert`, reusing the :977-991 microbench style)

**Interfaces:**
- Consumes: everything above.
- Produces: the proven walking-skeleton thread; a measured seal/wrap/unwrap number against the Bet-B 4 ms p95 budget.

- [ ] **Step 1: Write the failing E2E scenario**

Add to `clinical_pull.rs` (mirroring its existing test's setup — schema load on both DBs, node keys, server spawn, puller run):

```rust
#[tokio::test]
async fn sealed_medication_syncs_with_custody_then_shred_propagates() {
    // Skip unless CAIRN_TEST_PG && CAIRN_TEST_PG2.
    // A: seal-submit a medication assert (medication::assert_medication) →
    //    statement projected on A.
    // Serve A; pull into B with B's key (real binary, unwrap_cert flows).
    // Assert on B: event_log row sealed=true; event_dek row present (B's own
    //   re-wrap opens with B's secret); event_clear + medication_statement
    //   projected — A→B projection equality for the sealed event.
    // A: shred::shred_event(target, "patient request — no retention basis").
    // Pull again into B.
    // Assert on B: tombstone applied; erasure_shred_log row; event_dek,
    //   event_clear, medication_statement all scrubbed; both event_log rows
    //   intact and cairn_verify still true for the sealed row.
    // RESTORE HALF: wipe B entirely (drop/recreate schema the way the existing
    //   clinical_pull test resets), full-sweep pull from A again →
    //   B converges to: sealed row admitted, NO custody (server excludes
    //   shredded DEKs), NO projection, tombstone + shred log present.
    //   Set-union re-delivery resurrects NOTHING (§3.8 restore-replays-shred).
}
```

Write it in full against the existing harness helpers in that file (reuse its spawn/pull functions; read the file first and mirror its patterns exactly).

- [ ] **Step 2: Run to verify it fails, then iterate to green**

Run: `CAIRN_TEST_PG=… CAIRN_TEST_PG2=… cargo test -p cairn-sync --test clinical_pull sealed_medication`
Debug wire/door gaps until PASS. Any door change discovered here gets its own pinned regression test in the corresponding task's test file.

- [ ] **Step 3: bench-seal**

Add a `bench-seal` subcommand: seal_event_payload + wrap_dek_for + unwrap_dek + unseal_event_payload over a ~1.5 KB representative medication payload, N=10_000, print ns/op per stage (house rule 6: derive fixtures). Run it on the dev box, paste the numbers into the ADR-0052 "Consequences" measurement note (and they feed the deferred per-episode-hierarchy question).

Run: `cargo run -p cairn-sync -- bench-seal`
Expected: prints four per-stage numbers; sanity ceiling: whole pipeline well under 1 ms/event on x86 (it is microseconds-scale AEAD over 1.5 KB).

- [ ] **Step 4: Full workspace + commit**

```bash
CAIRN_TEST_PG=… CAIRN_TEST_PG2=… CAIRN_TEST_PG3=… cargo test --workspace
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
git add crates/cairn-sync docs/spec/decisions/0052-*.md
git commit -m "feat(#189,#92): E2E — sealed sync with custody, shred propagation, restore-replays-shred; seal bench numbers into ADR-0052"
```

---

### Task 14: Docs regen, follow-up issues, PR

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (new slice entry; prune per house rule 8)

- [ ] **Step 1: File follow-up issues** (`gh issue create`, each linking #189/#92 + ADR-0052):
1. Unwrap-cert trust pinning: serve side must check the cert kid against the node-plane trust set (skeleton verifies signature only).
2. Sequester (custody narrowing) + sensitivity-stream code + safety-projection sibling emission — the confidentiality layer on top of erasability (ADR-0006 mechanism; the skeleton built the substrate).
3. Unwrap-key rotation ceremony (re-wrap all custody).
4. Blob-byte born-sealing (ADR-0013 inheritance).
5. Shred authorization policy hooks (who may shred what is policy; mechanism executes any floor-valid tombstone — needs the policy-assertion wiring, ADR-0024).
6. FTS/RAG surface: when full-text lands, it builds on `event_clear` ONLY, with shred-triggered invalidation pinned by test (#92 (b) enforcement for real indexes).

- [ ] **Step 2: Update HANDOVER.md + ROADMAP.md**

HANDOVER: mark #189/#92 DONE in the review-course section (P3 closed if #204 scheduling stands), new session block (condense, keep < 500 lines). ROADMAP: add the slice entry (next number after Slice 41) with the one-paragraph summary + PR link.

- [ ] **Step 3: Verify everything, push, open the PR**

```bash
CAIRN_TEST_PG=… CAIRN_TEST_PG2=… CAIRN_TEST_PG3=… cargo test --workspace   # expect ~720+/0
uv run --with-requirements docs/requirements.txt -- mkdocs build
git push -u origin feat/adr-0052-born-sealed-189-92
gh pr create --title "feat(#189,#92): ADR-0052 — born-sealed clinical bodies + walking-skeleton seal slice" \
  --body "Closes #189. Closes #92. <summary of the posture decision, the custody plane, the shred thread, the E2E proof, the follow-up issues filed, and the dev-rig-wipe operational caveat>

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```

---

## Self-review notes (already applied)

- **Spec coverage:** posture floor (Task 7), twin-under-seal (Tasks 2/7), custody plane + backup-can't-reconstruct (Tasks 3/5), FTS/RAG invalidation (structural: `event_clear` is the only clear surface, Task 5 + follow-up issue 6), safety-projection/descriptor/rung-2 prose (Task 1), shred + restore-replays-shred (Tasks 5/8/9/13), lenient apply (Task 8), all-verbs seal-at-write (Task 10), shred ceremony (Task 11), sidecar (Task 12), bench (Task 13).
- **Known judgment calls the implementer must NOT reopen silently:** born-sealed floor keyed on the `clinical.%` type prefix; `event_clear` populated before `event_log` insert (no FK, same txn); apply door warns-and-admits on a bad sidecar DEK; `medication_cessation` scrub only if it carries `content_address` (else defer + note in ADR).
- **Type consistency:** door = `submit_event(bytea, bytea, bytea, bytea)` / `apply_remote_event(bytea, bytea, bytea, bytea)`; helper = `cairn_clear_payload(event_log)`; container keys `sealed/alg/nonce/ct`; inner keys `payload/plaintext_twin`; wrapped-DEK blob = 104 bytes.
