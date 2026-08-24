# DR Slice 1 — The Independent Unwrap Key Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Break the coupling that makes a node's data custody die with its identity, so a restored solo node can open the sealed clinical bodies it inherits (#495).

**Architecture:** The node's X25519 DEK-unwrap key stops being HKDF-derived from the Ed25519 signing seed and becomes an independent keypair, sealed at rest under the same operator secrets and carried across a restore in the `CAIRNL1` local-state export. Registration of its public half moves from an implicit per-write side effect to an explicit provisioning act, and the write path verifies rather than registers.

**Tech Stack:** Rust (cairn-event, cairn-node, cairn-sync), PostgreSQL 18 + `cairn_pgx`, `x25519-dalek`, `argon2`, `chacha20poly1305`, `ciborium`.

**Spec:** [docs/superpowers/specs/2026-08-24-dr-clinical-tier-recovery-design.md](../specs/2026-08-24-dr-clinical-tier-recovery-design.md)

## Global Constraints

- **AGPL-3.0** for all code; every dependency must be AGPL-3.0-compatible, checked *before* adding. This slice adds **no new dependency**.
- **TDD**: the failing test comes first, always. No production code without a test that drove it.
- **House rule 6 — never hard-code cryptographic material in tests.** Derive every key, seed, salt and nonce at runtime (`std::array::from_fn(|i| …)` or a helper). A literal trips CodeQL's `rust/hard-coded-cryptographic-value` and blocks the scan (#146).
- **Inline documentation for a junior developer** on every non-trivial function: *why* it exists and how it fits, not what the next line does.
- **Files under 500 lines where feasible.** `keystore.rs` is 315 and gains ~90; `localstate.rs` is 673 and is already over — do not grow it without extracting; see Task 6.
- **`SCHEMA_GENERATION` stays 50.** This slice adds no migration. Slice 2 adds `db/051` and bumps it.
- **Run the DB-gated suites via `scripts/run-db-gated-tests.sh`.** Without `CAIRN_TEST_PG`/`PG2`/`PG3` the DB suites self-skip and cargo counts them as passed; since #450 a run without them FAILS unless `CAIRN_ALLOW_DB_SKIP=1` is set affirmatively.
- **Take the guard before connecting**: `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
- **UUIDs bind as text**: cairn-node does not enable tokio-postgres's `with-uuid-1`. Bind `&uuid.to_string()` and cast in SQL (`$1::text::uuid`).
- **A live IDE contends on the shared `target/` lock.** If a narrow `cargo test` hangs before compiling, use a scratch `CARGO_TARGET_DIR=/tmp/cairn-slice1`, never kill the IDE.

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/cairn-event/src/seal.rs` | The seal/wrap crypto core | Add `generate_unwrap_secret`; re-document `derive_unwrap_secret` as migration-only |
| `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs` | Source guard: no production site may re-couple identity to custody | **Create** |
| `crates/cairn-node/src/keystore.rs` | Key material at rest | Add the `node.unwrap` file lifecycle; delete `unwrap_secret(sk)` |
| `crates/cairn-node/src/medication/sealed_submit.rs` | Seal-then-sign write path | `ensure_unwrap_key` becomes a verification, not a registration |
| `crates/cairn-node/src/medication/{reconciliation,attestation,signoff}.rs` | Other write paths | Drop the now-unused argument at 3 call sites |
| `crates/cairn-node/src/localstate.rs` | The `CAIRNL1` export | Add the `unwrap_secret` slot + the `episode_deks` producer |
| `crates/cairn-node/src/localstate_read.rs` | The DB-reading producer | **Create** (keeps `localstate.rs` from growing past its already-over budget) |
| `crates/cairn-node/src/main.rs` | CLI | `init` mints; new `establish-unwrap-key`; `status` reports; restore installs |
| `crates/cairn-sync/src/main.rs` | Sync daemon | Fail fast when its derived key disagrees with the registered one |
| `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` | The pins | Invert three; update the mechanism test's doc |
| `docs/spec/decisions/0066-*.md`, `docs/spec/*.md` | The decision record | **Create** / revise |

---

### Task 1: ADR-0066 and the spec revision

Write the decision before the code that cites it — every comment added later points here.

**Files:**
- Create: `docs/spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md`
- Modify: `docs/spec/decisions/README.md` (the ADR index), `docs/spec/index.md` (version line v0.67 → v0.68)
- Modify: `docs/spec/security.md` §7.10 (durability/DR) and `docs/spec/data-model.md` §3.8 (erasure/custody) — add the custody-lifecycle paragraph and link the ADR

- [ ] **Step 1: Write the ADR**

Follow the house skeleton exactly (see `0065-narrow-the-custody-never-the-reach.md`): `# ADR-0066 — Identity dies with the disk; custody must not`, then **Status: Accepted**, **Date: 2026-08-24**, **Derives from**, **Applies** (principles 1, 4, 9, 12; ADR-0005, ADR-0011, ADR-0026, ADR-0052, ADR-0056), **Canonical spec home**, **Context**, numbered **Decisions**, **Consequences**, **Rejected alternatives**.

The decisions, stated so they can be cited from code comments:

1. **The node unwrap key is an independent X25519 keypair**, not HKDF-derived from the signing seed. Supersedes ADR-0052 decision 4's derivation clause only; its rationale (no second operator ceremony) is preserved — the same op-passphrase and recovery code seal it.
2. **ADR-0026 decision 4 stands unchanged.** The signing key is still never backed up, and now nothing depends on it surviving.
3. **The unwrap secret rides the `CAIRNL1` export**, which ADR-0026 point 3 already defines for non-event, non-signing-key material. It yields read access, never a signing identity — point 4's stated test.
4. **A restored node adopts the exported unwrap key rather than minting one**, because `node_unwrap_key` is a singleton whose registrar refuses a differing key (db/037). No keyring exists and none is needed.
5. **Existing nodes adopt their currently-derived secret** as their first independent key — lossless, no rewrap. The path works only while the derived secret is still reconstructible.
6. **Registering the public half is a provisioning act, not a write-path side effect.** The write path verifies and fails loudly; see Task 4's rationale.
7. **The export excludes a shredded event's DEK** (ADR-0026 point 6). A restore must never resurrect an erasure the node already executed.

Record honestly in **Consequences**: the export becomes authorization-relevant in slice 2 (it will carry the actor registry); and cairn-sync still derives its own unwrap secret, which Task 5 makes loud rather than silent.

- [ ] **Step 2: Verify the docs build**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: exits 0, no warnings about the new file. Use the pinned requirements — never an ad-hoc `--with` install.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md docs/spec/decisions/README.md docs/spec/index.md docs/spec/security.md docs/spec/data-model.md
git commit -m "docs(#495): ADR-0066 — identity dies with the disk; custody must not"
```

---

### Task 2: `generate_unwrap_secret`, and the guard that keeps the derivation contained

**Files:**
- Modify: `crates/cairn-event/src/seal.rs` (add the generator near `derive_unwrap_secret`, line ~194)
- Create: `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`

**Interfaces:**
- Produces: `cairn_event::seal::generate_unwrap_secret() -> Result<Zeroizing<[u8; 32]>, EventError>`

- [ ] **Step 1: Write the failing test**

In `crates/cairn-event/src/seal.rs`, inside the existing `#[cfg(test)] mod tests`:

```rust
/// A generated unwrap secret is a first-class wrap recipient, and is tied to no seed.
/// This is the property ADR-0066 decision 1 rests on: two calls must differ (so the key
/// is not a function of anything), and the resulting keypair must behave exactly as a
/// derived one did at the wrap/unwrap boundary (so nothing downstream can tell them
/// apart). House rule 6: every byte here comes from the generator or an existing runtime
/// fixture — no literals.
#[test]
fn a_generated_unwrap_secret_is_independent_and_wraps_like_a_derived_one() {
    let a = generate_unwrap_secret().unwrap();
    let b = generate_unwrap_secret().unwrap();
    assert_ne!(*a, *b, "two generated unwrap secrets must differ");

    let dek = dek_fixture();
    let wrapped = wrap_dek_for(&dek, &unwrap_public(&a)).unwrap();
    assert_eq!(
        unwrap_dek(&wrapped, &a).unwrap().as_slice(),
        &dek,
        "the generated keypair must open its own wrap"
    );
    assert!(
        unwrap_dek(&wrapped, &b).is_err(),
        "a different secret must not open it"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p cairn-event --lib seal::tests::a_generated_unwrap_secret -- --nocapture`
Expected: FAIL — `cannot find function 'generate_unwrap_secret' in this scope`.

- [ ] **Step 3: Implement the generator**

In `crates/cairn-event/src/seal.rs`, immediately **above** `derive_unwrap_secret`:

```rust
/// Mint a fresh, INDEPENDENT X25519 unwrap secret (ADR-0066 decision 1).
///
/// WHY THIS EXISTS: the unwrap key used to be HKDF-derived from the node's Ed25519
/// signing seed, which tied DATA CUSTODY to NODE IDENTITY. ADR-0026 deliberately kills
/// the identity on disaster recovery (the signing key is never backed up), so the
/// derivation silently killed the custody too — every inherited `event_dek` row became
/// unopenable on a restored solo node (#495). An independent key has its own lifecycle:
/// it is sealed at rest under the same operator secrets and carried across a restore in
/// the local-state export, so identity can die without taking the record with it.
///
/// The raw 32 bytes need no clamping here — `x25519_dalek::StaticSecret` clamps on use,
/// exactly as it did for the HKDF output this replaces.
pub fn generate_unwrap_secret() -> Result<Zeroizing<[u8; 32]>, EventError> {
    let mut out = Zeroizing::new([0u8; 32]);
    getrandom::fill(out.as_mut())
        .map_err(|e| EventError::Seal(format!("entropy failure: {e}")))?;
    Ok(out)
}
```

- [ ] **Step 4: Run the test again**

Run: `cargo test -p cairn-event --lib seal::tests::a_generated_unwrap_secret`
Expected: PASS.

- [ ] **Step 5: Re-document the derivation as migration-only**

Replace `derive_unwrap_secret`'s doc comment (keep the body byte-for-byte — the migration in Task 4 depends on it computing exactly what it always did):

```rust
/// ⚠️ **MIGRATION PATH ONLY — DO NOT CALL THIS TO OBTAIN A NODE'S UNWRAP SECRET.**
/// Use `cairn_node::keystore::load_unwrap_secret`, which reads the independent key
/// ADR-0066 decision 1 established.
///
/// This function survives for exactly one purpose: a node provisioned BEFORE ADR-0066
/// has `event_dek` rows wrapped to the public half of the secret this derives, so
/// `cairn-node establish-unwrap-key` re-derives it once and adopts it as that node's
/// first independent key. That adoption is lossless — no row is rewrapped — and it
/// works only while the signing seed still reconstructs the old secret.
///
/// **Calling it anywhere else re-creates the coupling that cost the whole clinical
/// record on a restored solo node (#495).** `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`
/// pins the production call sites; the count failing is the guard working.
///
/// Deterministic in the seed, and cryptographically independent of the seed's use as a
/// signing key: the distinct HKDF info tag means recovering one teaches nothing about
/// the other.
pub fn derive_unwrap_secret(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {
```

- [ ] **Step 6: Write the source guard**

Create `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`. Model it on the existing source-derived guards (`no_drugref_dependency.rs`, `event_log_row_by_name.rs`) and reuse their `sources` helper:

```rust
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

use std::path::Path;

mod common;
use common::sources;

/// Production files permitted to call `derive_unwrap_secret`, with the reason each is
/// allowed. A file NOT on this list calling it is the failure this guard exists for.
const ALLOWED: &[(&str, &str)] = &[(
    "crates/cairn-node/src/keystore.rs",
    "the ADR-0066 adoption migration (`adopt_derived_unwrap_secret`) — the one place a \
     pre-ADR-0066 node re-derives its old secret to keep its existing event_dek rows openable",
)];

#[test]
fn only_the_adoption_migration_derives_the_unwrap_secret() {
    let root = sources::repo_root();
    let mut offenders: Vec<String> = Vec::new();

    for path in sources::production_rust_files(&root) {
        let text = sources::read_source(&path);
        if !text.contains("derive_unwrap_secret") {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
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

    // Anti-vacuity: the guard must be scanning real files, not an empty set. If the
    // helper's globbing breaks, an empty sweep would pass silently and forever.
    assert!(
        sources::production_rust_files(&root).count() > 50,
        "the production-source sweep found almost nothing — the scan itself is broken, and \
         a guard that inspects nothing always passes"
    );
}
```

**If `sources` has no `production_rust_files` helper, add one** that walks `crates/*/src/**/*.rs` and skips `#[cfg(test)]`-only files by path (`/tests/`, `/benches/`). Remember the repo trap: a new `pub fn` in `crates/cairn-node/tests/common/mod.rs` must ALSO be added to `identity_scaffolding_shared.rs`'s hand-written expected-helper array, or `derivation_finds_the_expected_helpers` fails.

- [ ] **Step 7: Run the guard — it must pass only after Task 4**

Run: `cargo test -p cairn-node --test unwrap_secret_is_not_derived`
Expected at this point: **FAIL**, naming `crates/cairn-node/src/medication/sealed_submit.rs` and `crates/cairn-sync/src/main.rs`. That is correct — those are the live couplings Tasks 4 and 5 remove. Leave it red and proceed; it goes green at the end of Task 5.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-event/src/seal.rs crates/cairn-node/tests/unwrap_secret_is_not_derived.rs
git commit -m "feat(#495): an independent unwrap secret, and the guard that keeps it uncoupled"
```

---

### Task 3: The `node.unwrap` file

**Files:**
- Modify: `crates/cairn-node/src/keystore.rs`

**Interfaces:**
- Consumes: `cairn_event::seal::generate_unwrap_secret` (Task 2)
- Produces:
  - `keystore::unwrap_key_path_for(key: &Path) -> PathBuf`
  - `keystore::generate_unwrap_sealed(path: &Path, op_pass: &str, recovery_code: &str) -> Result<[u8; 32], KeystoreError>` (returns the **public** half)
  - `keystore::write_unwrap_sealed(path: &Path, secret: &[u8; 32], op_pass: &str, recovery_code: &str) -> Result<(), KeystoreError>`
  - `keystore::load_unwrap_secret(path: &Path, secret: Option<&str>) -> Result<Zeroizing<[u8; 32]>, KeystoreError>`
  - `keystore::adopt_derived_unwrap_secret(sk: &SigningKey) -> Zeroizing<[u8; 32]>`
- Removes: `keystore::unwrap_secret(sk)` — every caller moves to `load_unwrap_secret`

- [ ] **Step 1: Write the failing tests**

Append to `keystore.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn unwrap_key_roundtrips_under_both_secrets_and_is_owner_only() {
    let dir = tempdir().unwrap();
    let p = dir.path().join("node.unwrap");
    let public = generate_unwrap_sealed(&p, "op", "REC-CODE").unwrap();

    let via_op = load_unwrap_secret(&p, Some("op")).unwrap();
    let via_rec = load_unwrap_secret(&p, Some("REC-CODE")).unwrap();
    assert_eq!(*via_op, *via_rec, "both secrets recover the same key");
    assert_eq!(
        cairn_event::seal::unwrap_public(&via_op),
        public,
        "the returned public half must match the sealed secret's"
    );
    assert!(
        matches!(load_unwrap_secret(&p, None), Err(KeystoreError::Sealed)),
        "a sealed unwrap key with no secret returns the distinct Sealed variant"
    );
}

#[test]
fn an_adopted_secret_still_opens_a_dek_wrapped_before_adoption() {
    // The migration promise (ADR-0066 decision 5): a node provisioned before ADR-0066 has
    // event_dek rows wrapped to its DERIVED public half. Adoption must keep every one of
    // them openable — that is what makes the migration lossless and rewrap-free.
    let dir = tempdir().unwrap();
    let p = dir.path().join("node.unwrap");
    let (sk, _kid) = cairn_event::generate_key().unwrap();

    // A DEK wrapped the OLD way, before adoption. House rule 6: derived at runtime.
    let old_secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
    let dek: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3));
    let wrapped =
        cairn_event::seal::wrap_dek_for(&dek, &cairn_event::seal::unwrap_public(&old_secret))
            .unwrap();

    let adopted = adopt_derived_unwrap_secret(&sk);
    write_unwrap_sealed(&p, &adopted, "op", "REC-CODE").unwrap();

    let loaded = load_unwrap_secret(&p, Some("op")).unwrap();
    assert_eq!(
        cairn_event::seal::unwrap_dek(&wrapped, &loaded)
            .expect("an adopted key must open a pre-adoption wrap")
            .as_slice(),
        &dek,
        "adoption is lossless: no event_dek row needs rewrapping"
    );
}

#[test]
fn unwrap_key_at_rest_reports_missing_sealed_and_corrupt() {
    let dir = tempdir().unwrap();
    assert!(matches!(
        key_at_rest_state(&dir.path().join("nope.unwrap")),
        KeyAtRest::Missing
    ));
    let p = dir.path().join("node.unwrap");
    generate_unwrap_sealed(&p, "op", "REC-CODE").unwrap();
    assert!(matches!(
        key_at_rest_state(&p),
        KeyAtRest::Sealed { dual_recipient: true }
    ));
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p cairn-node --lib keystore::tests`
Expected: FAIL — `cannot find function 'generate_unwrap_sealed'`.

- [ ] **Step 3: Implement**

Add to `keystore.rs`, replacing the existing `unwrap_secret` function:

```rust
/// The unwrap-key file for a signing-key path: `<key>.unwrap`, a sibling — discoverable
/// from what every command already has, exactly like the `.lsk` sidecar. Pure.
pub fn unwrap_key_path_for(key: &Path) -> PathBuf {
    let mut name = key
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".unwrap");
    key.with_file_name(name)
}

/// Mint this node's INDEPENDENT X25519 unwrap secret and write it sealed (mode 0600)
/// under both operator secrets. Returns the PUBLIC half, which the caller registers in
/// the database (`cairn_register_unwrap_key`) — the secret half never enters the DB, so a
/// database backup alone can never unwrap a DEK.
///
/// The sealed format is the SAME dual-recipient bundle the signing key uses: both are
/// 32-byte secrets, so `seal::seal` covers this with no new ceremony and no new format
/// (ADR-0066 decision 1 — the operator still holds one passphrase and one recovery code).
pub fn generate_unwrap_sealed(
    path: &Path,
    op_pass: &str,
    recovery_code: &str,
) -> Result<[u8; 32], KeystoreError> {
    let secret = cairn_event::seal::generate_unwrap_secret()
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_unwrap_sealed(path, &secret, op_pass, recovery_code)?;
    Ok(cairn_event::seal::unwrap_public(&secret))
}

/// Write a KNOWN unwrap secret sealed under both operator secrets. Two callers, both
/// carrying a secret that must not change: the ADR-0066 adoption migration (which keeps a
/// pre-ADR-0066 node's existing `event_dek` rows openable) and `restore`, which installs
/// the dead node's unwrap secret so the restored node inherits its custody.
pub fn write_unwrap_sealed(
    path: &Path,
    secret: &[u8; 32],
    op_pass: &str,
    recovery_code: &str,
) -> Result<(), KeystoreError> {
    let material = zeroize::Zeroizing::new(*secret);
    let sealed = seal::seal(&material, op_pass, recovery_code)
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    crate::fsio::atomic_write(path, &seal::to_cbor(&sealed), Some(0o600))?;
    Ok(())
}

/// Load the node's unwrap secret, auto-detecting sealed vs plaintext exactly as [`load`]
/// does for the signing key — including the distinct [`KeystoreError::Sealed`] variant, so
/// the CLI can prompt for the passphrase from ONE load attempt with no TOCTOU-prone
/// pre-classification read.
pub fn load_unwrap_secret(
    path: &Path,
    secret: Option<&str>,
) -> Result<zeroize::Zeroizing<[u8; 32]>, KeystoreError> {
    let bytes = zeroize::Zeroizing::new(std::fs::read(path)?);
    if let Ok(sealed) = seal::from_cbor(&bytes) {
        let secret = secret.ok_or(KeystoreError::Sealed)?;
        seal::unseal(&sealed, secret).ok_or_else(|| {
            KeystoreError::Key(
                "cannot unseal the unwrap key: wrong passphrase/recovery code or corrupt file"
                    .into(),
            )
        })
    } else {
        Ok(zeroize::Zeroizing::new(
            bytes.as_slice().try_into().map_err(|_| {
                KeystoreError::Key("not a sealed bundle and not a 32-byte unwrap secret".into())
            })?,
        ))
    }
}

/// ADR-0066 decision 5 — THE MIGRATION, and the only production caller of
/// `derive_unwrap_secret`.
///
/// A node provisioned before ADR-0066 wrapped every `event_dek` row to the public half of
/// the secret HKDF-derived from its signing seed. Re-deriving it once and adopting it as
/// that node's first INDEPENDENT key keeps all of them openable — no rewrap, no migration
/// of custody rows, nothing to get wrong at 3am. It works only while the signing seed
/// still reconstructs the old secret, which is why this migration is cheap now and never
/// cheaper.
pub fn adopt_derived_unwrap_secret(sk: &SigningKey) -> zeroize::Zeroizing<[u8; 32]> {
    let seed = zeroize::Zeroizing::new(sk.to_bytes());
    cairn_event::seal::derive_unwrap_secret(&seed)
}
```

Add `use std::path::PathBuf;` to the imports.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cairn-node --lib keystore::tests`
Expected: PASS, all of them. The crate will not yet build its binary — `sealed_submit.rs` still calls the deleted `unwrap_secret`; Task 4 fixes that.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/keystore.rs
git commit -m "feat(#495): the node.unwrap keystore file, with a lossless adoption path"
```

---

### Task 4: Registration becomes provisioning; the write path verifies

**Why this shape** (read before editing): `ensure_unwrap_key(client, node_sk)` currently derives the secret and registers its public half on *every* sealed write. With an independent key the write path has no way to reach the file, and threading it through would cascade into all 8 `seal_sign_submit` call sites and their callers. It is also the wrong shape: a node's custody key is a **provisioned fact**, not an implicit side effect of the first write. So registration moves to `init` / `establish-unwrap-key`, and the write path *verifies* — turning "silently register whatever key the signer implies" into "writing without a provisioned custody key fails loudly."

**Files:**
- Modify: `crates/cairn-node/src/medication/sealed_submit.rs:65-80` (`ensure_unwrap_key`), `:311`
- Modify: `crates/cairn-node/src/medication/reconciliation.rs:254`, `attestation.rs:206`, `signoff.rs:156`
- Modify: `crates/cairn-node/src/main.rs` (`init` arm; new `EstablishUnwrapKey` subcommand; `status`)

**Interfaces:**
- Produces: `ensure_unwrap_key(client: &tokio_postgres::Client) -> anyhow::Result<()>` — **one fewer parameter**

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/unwrap_key_provisioning.rs`:

```rust
//! ADR-0066 decision 6 — the node's custody key is provisioned, and the write path checks.

mod common;
use cairn_node::db;

#[tokio::test]
async fn a_sealed_write_fails_loudly_when_no_unwrap_key_is_registered() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // A node with NO registered unwrap key — the state a fresh database is in before
    // `init` / `establish-unwrap-key` runs.
    c.execute("DELETE FROM node_unwrap_key", &[]).await.unwrap();

    let err = cairn_node::medication::sealed_submit::ensure_unwrap_key(&c)
        .await
        .expect_err("an unprovisioned node must refuse, not silently write without custody");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("establish-unwrap-key"),
        "the refusal must name the remedy the operator can actually run; got: {msg}"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `scripts/run-db-gated-tests.sh` is the full sweep; for this one test use the connection string directly:
`CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=postgres dbname=cairn_test" cargo test -p cairn-node --test unwrap_key_provisioning`
Expected: FAIL to compile — `ensure_unwrap_key` takes 2 arguments.

- [ ] **Step 3: Rewrite `ensure_unwrap_key` as a verification**

In `sealed_submit.rs`, replace the whole function (docs included):

```rust
/// Verify this node's X25519 public unwrap key is registered before a sealed write.
///
/// ADR-0066 decision 6: registering it is a PROVISIONING act (`cairn-node init` /
/// `establish-unwrap-key`), not a side effect of the first write. Before ADR-0066 this
/// function derived the secret from the signing key and registered it on every write,
/// which quietly meant "whatever key this signer implies is this node's custody key" —
/// and tied custody to identity, the coupling that emptied a restored node's clinical
/// record (#495).
///
/// The door needs the key committed and visible so it can wrap this event's DEK into
/// recoverable custody. A node without one would write sealed bodies it can never
/// crypto-shred, so this refuses rather than degrades — and names the remedy, because a
/// refusal an operator cannot act on is not a safety control.
pub async fn ensure_unwrap_key(client: &tokio_postgres::Client) -> anyhow::Result<()> {
    let registered: bool = client
        .query_one("SELECT EXISTS (SELECT 1 FROM node_unwrap_key)", &[])
        .await?
        .get(0);
    anyhow::ensure!(
        registered,
        "this node has no registered unwrap key, so a sealed body could not be given \
         recoverable custody — run `cairn-node establish-unwrap-key` (ADR-0066)"
    );
    Ok(())
}
```

- [ ] **Step 4: Update the four call sites**

`sealed_submit.rs:311`, `reconciliation.rs:254`, `attestation.rs:206`, `signoff.rs:156` — drop the second argument: `ensure_unwrap_key(client).await?;`. If `node_sk` becomes unused in any of those functions, do **not** delete the parameter — it is still the signing key those paths use elsewhere; let the compiler tell you, and only remove a binding it flags.

- [ ] **Step 5: Wire `init` and add `establish-unwrap-key`**

In `main.rs`, in the `Init` arm, immediately after the signing key is minted and the recovery code is known:

```rust
// ADR-0066: mint the node's INDEPENDENT unwrap key beside the signing key and register
// its public half. Custody is provisioned here so it never depends on who happens to
// sign the first clinical event — and so it survives a disaster the identity does not.
let unwrap_path = cairn_node::keystore::unwrap_key_path_for(&cli.key);
let unwrap_pub =
    cairn_node::keystore::generate_unwrap_sealed(&unwrap_path, &op_pass, &recovery_code)?;
db.execute("SELECT cairn_register_unwrap_key($1)", &[&unwrap_pub.as_slice()])
    .await?;
eprintln!("unwrap key established at {}", unwrap_path.display());
```

Add the subcommand to `enum Cmd`:

```rust
/// Establish this node's independent unwrap key (ADR-0066), adopting the previously
/// derived one if this node predates the split. Idempotent.
EstablishUnwrapKey {
    #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
    passphrase: Option<String>,
},
```

Its arm — the migration, and the one place `adopt_derived_unwrap_secret` is called:

```rust
Cmd::EstablishUnwrapKey { passphrase } => {
    let unwrap_path = cairn_node::keystore::unwrap_key_path_for(&cli.key);
    let op = resolve_passphrase(passphrase)?;
    let sk = cairn_node::keystore::load(&cli.key, Some(&op))?;

    // Idempotent: an existing file is loaded, not replaced. Overwriting it would mint a
    // key that opens nothing, and `cairn_register_unwrap_key` would then refuse the swap
    // — leaving the node unable to unwrap its own custody with no way back.
    let secret = if unwrap_path.exists() {
        cairn_node::keystore::load_unwrap_secret(&unwrap_path, Some(&op))?
    } else {
        // ADR-0066 decision 5: adopt the secret this node's event_dek rows are already
        // wrapped to, so every existing sealed body stays openable without a rewrap.
        let adopted = cairn_node::keystore::adopt_derived_unwrap_secret(&sk);
        let code = cairn_node::seal::mint_recovery_code()?;
        cairn_node::keystore::write_unwrap_sealed(&unwrap_path, &adopted, &op, &code)?;
        println!("unwrap-key recovery code (write this down, it is shown once): {code}");
        adopted
    };

    let public = cairn_event::seal::unwrap_public(&secret);
    db.execute("SELECT cairn_register_unwrap_key($1)", &[&public.as_slice()])
        .await?;
    println!("unwrap key established at {}", unwrap_path.display());
}
```

Check the actual name of the recovery-code minting helper used by `init` and use that one — do not invent a second generator.

In the `status` arm, add a line beside the existing key-at-rest report:

```rust
println!(
    "unwrap key: {:?} ({})",
    cairn_node::keystore::key_at_rest_state(&cairn_node::keystore::unwrap_key_path_for(&cli.key)),
    if unwrap_registered { "registered" } else { "NOT REGISTERED — run `cairn-node establish-unwrap-key`" }
);
```

reading `unwrap_registered` from `SELECT EXISTS (SELECT 1 FROM node_unwrap_key)`.

- [ ] **Step 6: Run the test and the crate**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=postgres dbname=cairn_test" cargo test -p cairn-node --test unwrap_key_provisioning`
Expected: PASS.
Run: `cargo test -p cairn-node --all-targets` (`--all-targets`, not `--lib`: a `cfg(test)`-only import regression only fails the integration build under `-D warnings`).
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/medication crates/cairn-node/src/main.rs crates/cairn-node/tests/unwrap_key_provisioning.rs
git commit -m "feat(#495): custody is provisioned, not a write-path side effect"
```

---

### Task 5: Make cairn-sync's divergence loud, and file what this slice cannot fix

cairn-sync derives its own unwrap secret at two live sites (`main.rs:3043`, `:5050`) and does **not** depend on cairn-node, so it cannot read the new keystore file. After Task 4 a freshly-`init`ed node registers an independent key while cairn-sync still derives one from its signing key — they disagree, and `rewrap_custody_for_peer` would fail to open local DEKs. This task makes that disagreement fail fast instead of degrading quietly, and files the real fix.

**Files:**
- Modify: `crates/cairn-sync/src/main.rs` (the startup check, beside the existing `cairn_pgx_version()` fail-fast)

- [ ] **Step 1: Write the failing test**

In `crates/cairn-sync/src/main.rs`'s test module, over the pure predicate:

```rust
/// ADR-0066: cairn-sync still DERIVES its unwrap secret while cairn-node now loads an
/// independent one. When they disagree the serve arm cannot open this node's own custody,
/// so it must stop and say so — degrading silently would look exactly like a peer that
/// simply has no custody to offer.
#[test]
fn a_divergent_unwrap_key_is_detected_rather_than_degraded() {
    let mine: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_add(1));
    let other: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_add(9));
    assert!(unwrap_key_matches(&mine, Some(&mine)), "agreement passes");
    assert!(!unwrap_key_matches(&mine, Some(&other)), "divergence is caught");
    assert!(
        unwrap_key_matches(&mine, None),
        "an unregistered key is not a divergence — nothing has been claimed yet"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p cairn-sync a_divergent_unwrap_key`
Expected: FAIL — `cannot find function 'unwrap_key_matches'`.

- [ ] **Step 3: Implement the predicate and the startup check**

```rust
/// True iff this daemon's unwrap public half agrees with what the database has registered.
/// `None` (nothing registered) is NOT a divergence: the node has claimed no custody key
/// yet. Pure, so the decision is testable without a database.
fn unwrap_key_matches(mine: &[u8; 32], registered: Option<&[u8; 32]>) -> bool {
    match registered {
        None => true,
        Some(theirs) => mine == theirs,
    }
}
```

At the point cairn-sync already fails fast on a stale `cairn_pgx`, read `SELECT unwrap_pub FROM node_unwrap_key` and, on divergence, exit with:

```
this daemon derives unwrap key <hex[..16]> from its signing key, but the database has
<hex[..16]> registered (ADR-0066: cairn-node now provisions an INDEPENDENT unwrap key).
This daemon cannot open this node's custody. See issue #<N> — point --key at the same key
this node was provisioned with, or run cairn-sync against a node provisioned before ADR-0066.
```

- [ ] **Step 4: Run the test and the guard from Task 2**

Run: `cargo test -p cairn-sync a_divergent_unwrap_key` → PASS.
Run: `cargo test -p cairn-node --test unwrap_secret_is_not_derived`
Expected: still FAIL, now naming only `crates/cairn-sync/src/main.rs`. Add that file to the guard's `ALLOWED` list with the reason *"pre-ADR-0066 derivation, fails fast on divergence — tracked by #<N>"*, then re-run.
Expected: PASS.

- [ ] **Step 5: File the two issues this slice cannot fix**

```bash
gh issue create --title "cairn-sync derives its unwrap secret instead of loading the node's provisioned one" --body "..."
gh issue create --title "cairn-sync's load_or_create_key silently overwrites a sealed node.key with a fresh plaintext one" --body "..."
```

The first: cairn-sync cannot read cairn-node's sealed keystore (no dependency, and `argon2` lives in cairn-node), so ADR-0066 leaves it deriving. Recommend extracting a small `cairn-keystore` crate both depend on. Note the interim fail-fast from this task.

The second, found while writing this plan and **unrelated to #495 — file it on its own merits**: `load_or_create_key` (`crates/cairn-sync/src/main.rs`) does `read_to_string`, and on failure **generates a new key and `std::fs::write`s it over the path**. cairn-node's sealed key file is binary CBOR, so `read_to_string` fails on invalid UTF-8 and the node's sealed signing key is destroyed by a daemon that was only trying to start. Suggested fix: refuse a file it cannot parse, never overwrite; create only when the path does not exist.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-sync/src/main.rs crates/cairn-node/tests/unwrap_secret_is_not_derived.rs
git commit -m "fix(#495): cairn-sync fails fast when its derived unwrap key diverges"
```

---

### Task 6: The export carries the secret and the surviving custody

This inverts three of the four pins. Note the `episode_deks` element shape is a CBOR struct inside the slot's opaque `Vec<u8>` leaf — exactly what "reserve the SLOT SHAPE without committing to the clinical tier's schema" was for, so the container format does not change.

**Files:**
- Create: `crates/cairn-node/src/localstate_read.rs` (`localstate.rs` is already 673 lines — do not grow it)
- Modify: `crates/cairn-node/src/localstate.rs` (the `unwrap_secret` slot; re-export the producer; correct the expired header)
- Modify: `crates/cairn-node/src/main.rs:349` (`seal_and_write_local_state_export` passes the loaded secret)
- Modify: `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` (invert pins)

**Interfaces:**
- Consumes: `keystore::load_unwrap_secret` (Task 3)
- Produces:
  - `localstate::EpisodeDek { event_id: String, dek_wrapped: Vec<u8> }` with `to_cbor`/`from_cbor` helpers
  - `localstate_read::read_local_state(db: &tokio_postgres::Client, unwrap_secret: Option<&[u8; 32]>) -> anyhow::Result<LocalState>`

- [ ] **Step 1: Write the failing test**

Add to `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs`:

```rust
/// ADR-0066 / #495 — the export now carries custody across the restore boundary.
///
/// Anti-vacuity, kept from the pin this replaces: the sealed event is authored through the
/// PRODUCTION door, so `event_dek` genuinely holds an openable wrapped DEK before the
/// export is built; and the recovered secret is checked to actually open it, not merely to
/// be present.
#[tokio::test]
async fn the_export_carries_the_unwrap_secret_and_the_surviving_dek() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (event_id, _bytes) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let node_secret = derive_unwrap_secret(&sk.to_bytes());
    // `&*` is load-bearing: `Some(&node_secret)` would be `Option<&Zeroizing<[u8; 32]>>`
    // — deref coercion does not reach inside `Some`, so deref explicitly.
    let exported = read_local_state(&c, Some(&*node_secret))
        .await
        .expect("export must succeed");

    let carried = exported
        .unwrap_secret
        .as_ref()
        .expect("ADR-0066: the export must carry the node's unwrap secret");
    assert_eq!(
        carried.as_slice(),
        node_secret.as_slice(),
        "the carried secret must be this node's, byte for byte"
    );

    let deks: Vec<EpisodeDek> = exported
        .episode_deks
        .iter()
        .map(|b| episode_dek_from_cbor(b).unwrap())
        .collect();
    let mine = deks
        .iter()
        .find(|d| d.event_id == event_id)
        .expect("the sealed event's custody must be in the export");

    // The whole point: the carried secret opens the carried DEK. Anything less proves
    // transport, not recovery.
    let recovered: [u8; 32] = carried.as_slice().try_into().unwrap();
    unwrap_dek(&mine.dek_wrapped, &recovered)
        .expect("the exported secret must open the exported custody row");
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=postgres dbname=cairn_test" cargo test -p cairn-node --test dr_clinical_guarantee_gap`
Expected: FAIL to compile — `read_local_state` takes 1 argument; no `unwrap_secret` field.

- [ ] **Step 3: Add the slot and the leaf type**

In `localstate.rs`, add to `LocalState` (keep `#[serde(default)]` — that is what makes it additive):

```rust
    /// ADR-0066: this node's INDEPENDENT X25519 unwrap secret, so a restored node inherits
    /// custody of every body it also inherits. The signing key is still deliberately absent
    /// (ADR-0026 point 4): a stolen, unsealed export must yield READ access, never a
    /// signing identity — and an unwrap secret is exactly read access.
    #[serde(default)]
    pub unwrap_secret: Option<Vec<u8>>,
```

and add it to `empty()` (as `None`) and to `is_empty()`.

```rust
/// One event's wrapped custody row, as it travels in `episode_deks`.
///
/// The slot's leaf type is opaque `Vec<u8>` by design, so each element is a small CBOR
/// struct rather than a format change. The DEK travels **wrapped**, exactly as it sits in
/// `event_dek`: the export never holds raw key material, and the separately-carried unwrap
/// secret is what opens it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeDek {
    pub event_id: String,
    pub dek_wrapped: Vec<u8>,
}

/// Serialize one custody row for the export slot. Pure.
pub fn episode_dek_to_cbor(d: &EpisodeDek) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(d, &mut out).expect("CBOR serialization of EpisodeDek cannot fail");
    out
}

/// Parse one custody row from the export slot. Errors, never panics.
pub fn episode_dek_from_cbor(bytes: &[u8]) -> Result<EpisodeDek, LocalStateError> {
    ciborium::from_reader(bytes).map_err(|e| LocalStateError::Decode(e.to_string()))
}
```

- [ ] **Step 4: Write the producer**

Create `crates/cairn-node/src/localstate_read.rs`:

```rust
//! ADR-0066 / #495 — the producer that fills the sealed local-state export.
//!
//! WHY A SEPARATE FILE: `localstate.rs` owns the FORMAT (container, seal, slots) and is
//! already past the 500-line house budget. This file owns the one thing that needs a
//! database — reading custody out of it — so neither grows the other.
//!
//! WHAT IT MUST NEVER DO: export a shredded event's DEK. ADR-0026 point 6 requires a
//! restore to honour an erasure the node already executed, and the export is the artifact
//! that crosses the restore boundary. `cairn_execute_shred` already deletes the custody
//! row, and `apply_remote_event` already refuses to re-create one for a shredded target
//! (db/020), so the filter below is a last line rather than the only one — kept because
//! the failure it prevents (resurrecting an erased body's key) is irreversible.

use crate::localstate::{episode_dek_to_cbor, EpisodeDek, LocalState};

/// Read this node's exportable local state.
///
/// `unwrap_secret` is the node's independent X25519 secret, loaded from the keystore by
/// the caller (it is not in the database and never will be — a DB backup that could
/// reconstruct a DEK would defeat the whole custody plane). `None` means the caller could
/// not load it; the export is still built, carrying custody rows that a restore will not
/// be able to open, and the caller must warn.
pub async fn read_local_state(
    db: &tokio_postgres::Client,
    unwrap_secret: Option<&[u8; 32]>,
) -> anyhow::Result<LocalState> {
    use anyhow::Context;

    let rows = db
        .query(
            "SELECT d.event_id::text AS event_id, d.dek_wrapped \
             FROM event_dek d \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM erasure_shred_log s WHERE s.target_event_id = d.event_id \
             ) \
             ORDER BY d.event_id",
            &[],
        )
        .await
        .context("reading event_dek custody for the local-state export")?;

    let episode_deks = rows
        .iter()
        .map(|r| {
            episode_dek_to_cbor(&EpisodeDek {
                event_id: r.get::<_, String>("event_id"),
                dek_wrapped: r.get::<_, Vec<u8>>("dek_wrapped"),
            })
        })
        .collect();

    Ok(LocalState {
        version: 1,
        node_default_deks: Vec::new(), // no node-default keystore exists yet (#495 promise 2)
        episode_deks,
        config: None, // no node config table exists yet
        drafts: Vec::new(), // no draft store exists yet
        unwrap_secret: unwrap_secret.map(|s| s.to_vec()),
    })
}
```

Delete the old `read_local_state` from `localstate.rs`, re-export the new one (`pub use crate::localstate_read::read_local_state;`) so existing call sites keep resolving, register the module in `lib.rs`, and **rewrite `localstate.rs`'s module header and the `LocalState` / `empty` / `is_empty` doc comments**: the expired-precondition warnings describe a hole that is now closed on the custody axis. Say what is still open (slice 2 lands the events and the registry) rather than deleting the warning wholesale — an over-correction here is how the original stale comment happened.

- [ ] **Step 5: Update the caller**

`main.rs`'s `seal_and_write_local_state_export` loads the unwrap secret and passes it, warning (never aborting — the export is optional and the medium is already written) when it cannot:

```rust
let unwrap_path = cairn_node::keystore::unwrap_key_path_for(key_path);
let unwrap = match cairn_node::keystore::load_unwrap_secret(&unwrap_path, Some(&op)) {
    Ok(s) => Some(s),
    Err(e) => {
        eprintln!(
            "WARNING: could not load the unwrap key at {} ({e}) — the export will carry \
             custody rows but no key to open them; a restore from it cannot read sealed \
             bodies. Run `cairn-node establish-unwrap-key`.",
            unwrap_path.display()
        );
        None
    }
};
let bundle = cairn_node::localstate::read_local_state(db, unwrap.as_deref()).await?;
```

Delete the `⚠️ #495 — THIS CEREMONY SUCCEEDS OVER AN EMPTY BUNDLE` warning block from that function's docs and replace it with what is now true.

- [ ] **Step 6: Invert the remaining pins**

In `dr_clinical_guarantee_gap.rs`:

- **Delete** `local_state_export_carries_no_dek_though_the_database_holds_one` — Step 1's test replaces it. Keep its `node_default_store == 0` assertion by moving it into the new test verbatim, with its comment: promise 2 still has no subject, and that is still worth pinning.
- **`export_carries_no_dek_for_the_survivor_and_none_for_the_shredded`**: invert the first assertion (the survivor's DEK **must** now be present) and leave the second exactly as written — the shredded event's key must still never appear. That asymmetry is the whole point of the test and it becomes load-bearing on this commit.
- **`the_only_local_state_producer_is_the_empty_constructor`**: the producer moved to `localstate_read.rs`, so this guard's file scan must now cover both files and expect **2** producers, naming them. Update the doc comment to say why.
- **`a_restored_nodes_fresh_seed_cannot_open_a_pre_restore_sealed_body`**: stays green — it describes the derivation's mechanics, which are unchanged. Update its doc to say the derivation is now migration-only, so a future reader does not mistake it for a description of the live path.
- **`medium_carries_the_federation_plane_and_no_clinical_event`**: untouched. It is #500's pin and slice 2 inverts it.

- [ ] **Step 7: Run the suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=postgres dbname=cairn_test" cargo test -p cairn-node --test dr_clinical_guarantee_gap`
Expected: PASS, with `medium_carries_the_federation_plane_and_no_clinical_event` still passing as a pin.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-node/src/localstate.rs crates/cairn-node/src/localstate_read.rs crates/cairn-node/src/lib.rs crates/cairn-node/src/main.rs crates/cairn-node/tests/dr_clinical_guarantee_gap.rs
git commit -m "feat(#495): the export carries the unwrap secret and surviving custody"
```

---

### Task 7: Restore installs the recovered key

**Files:**
- Modify: `crates/cairn-node/src/main.rs:1559-1590` (the restore arm's local-state block)
- Create: `crates/cairn-node/tests/restore_inherits_custody.rs`

- [ ] **Step 1: Write the failing test**

```rust
//! ADR-0066 — the promise this whole slice exists to make true: a node restored under a
//! FRESH identity can still open a body sealed before the disaster.

mod common;

#[test]
fn a_restored_node_opens_a_pre_restore_sealed_body() {
    let dir = tempfile::tempdir().unwrap();

    // The dead node: an independent unwrap key, and a DEK wrapped to it.
    let dead_key = dir.path().join("dead.unwrap");
    let dead_pub = cairn_node::keystore::generate_unwrap_sealed(&dead_key, "op", "REC").unwrap();
    let dek: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(11).wrapping_add(5));
    let wrapped = cairn_event::seal::wrap_dek_for(&dek, &dead_pub).unwrap();
    let dead_secret = cairn_node::keystore::load_unwrap_secret(&dead_key, Some("op")).unwrap();

    // The restored node: a DIFFERENT signing identity (ADR-0026 decision 4 mints one), and
    // the dead node's unwrap secret installed from the export under the NEW secrets.
    let restored_key = dir.path().join("restored.unwrap");
    cairn_node::keystore::write_unwrap_sealed(&restored_key, &dead_secret, "new-op", "NEW-REC")
        .unwrap();
    let restored_secret =
        cairn_node::keystore::load_unwrap_secret(&restored_key, Some("new-op")).unwrap();

    assert_eq!(
        cairn_event::seal::unwrap_dek(&wrapped, &restored_secret)
            .expect("ADR-0066: a restored node must open custody it inherited")
            .as_slice(),
        &dek,
        "identity died with the disk; custody did not"
    );
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p cairn-node --test restore_inherits_custody`
Expected: PASS immediately — Task 3 already built every piece. **This is deliberate**: the test states the slice's promise end-to-end over the real production functions, so it will redden if any later change breaks the chain. Note in its header that it is a promise test, not a TDD driver, so nobody mistakes it for a vacuous one.

- [ ] **Step 3: Install the secret in the restore arm**

Inside the `Some(plaintext)` branch, after `apply_local_state`:

```rust
match bundle.unwrap_secret.as_deref() {
    Some(bytes) => {
        let secret: [u8; 32] = bytes.try_into().map_err(|_| {
            anyhow::anyhow!(
                "the export's unwrap secret is {} bytes, not 32 — refusing to install a \
                 key that cannot open this node's inherited custody",
                bytes.len()
            )
        })?;
        let unwrap_path = cairn_node::keystore::unwrap_key_path_for(&cli.key);
        cairn_node::keystore::write_unwrap_sealed(
            &unwrap_path, &secret, &new_op_pass, &new_recovery_code,
        )?;
        let public = cairn_event::seal::unwrap_public(&secret);
        db.execute("SELECT cairn_register_unwrap_key($1)", &[&public.as_slice()])
            .await?;
        println!(
            "custody inherited: unwrap key installed at {} ({} episode DEK(s) carried)",
            unwrap_path.display(),
            bundle.episode_deks.len()
        );
        // Declared, and deliberately visible at the surface rather than buried in a
        // comment: the carried custody rows land with the clinical events, which the
        // medium does not yet carry (#500, slice 2).
        if !bundle.episode_deks.is_empty() {
            println!(
                "note: those {} custody row(s) are carried but not yet applied — they land \
                 with the clinical events (#500)",
                bundle.episode_deks.len()
            );
        }
    }
    None => eprintln!(
        "WARNING: the local-state export carries no unwrap key — sealed bodies restored \
         later will not be openable on this node (export predates ADR-0066?)"
    ),
}
```

- [ ] **Step 4: Fix #502 item 1 while you are on this line**

The enclosing `if let Ok(bytes) = std::fs::read(&export_path)` swallows every read error. Match on the kind: silent only on `NotFound`, warn on everything else, naming the path and the error — a `EACCES`/`EIO`/vanished mount currently renders identically to "no export was written", and the restore door fences closed behind the operator so there is no free second attempt.

- [ ] **Step 5: Run the crate**

Run: `cargo test -p cairn-node --all-targets`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/main.rs crates/cairn-node/tests/restore_inherits_custody.rs
git commit -m "feat(#495,#502): restore installs the inherited unwrap key"
```

---

### Task 8: The full gate, the docs, and the PR

- [ ] **Step 1: Run the whole local gate**

Run: `scripts/run-db-gated-tests.sh`
Expected: 0 failed. A warm `CARGO_TARGET_DIR` makes this ~15 min; cold it is ~2 h — start it and do Step 2 while it runs. A killed binary exits 101 with **zero** `test result: FAILED` lines, so check the exit code, not just the tail.

Run the workspace explicitly too — the cross-crate arity trap is real and per-crate runs miss it:
`cargo test --workspace --all-targets` (never `| tail`, which masks cargo's exit code).

- [ ] **Step 2: Update HANDOVER.md and ROADMAP.md**

HANDOVER's ⇒ NEXT still opens with the DR warning naming both #495 and #500 as open. Rewrite it: #495 closed by ADR-0066, #500 is the remaining half and slice 2 is the next build. Keep the reusable lesson (*a deferral is only honest while its stated precondition holds*) — it outlives the defect. Add the new traps a next session can spring:

- **`derive_unwrap_secret` is the adoption migration ONLY**, pinned by `unwrap_secret_is_not_derived.rs`; calling it elsewhere re-creates the #495 coupling.
- **Registering the unwrap key is provisioning, not a write-path side effect** (ADR-0066 decision 6) — a fresh DB under an existing node needs `establish-unwrap-key` before the first sealed write.
- **cairn-sync still derives and fails fast on divergence** — issue filed in Task 5.

ROADMAP: log the slice under the clinical-surface phase, name every issue opened and closed, and keep both files **under 500 lines** — condense, but never drop a live issue number while condensing (the PR #271 finding).

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin design/dr-clinical-tier-recovery-495-500
gh pr create --title "ADR-0066: identity dies with the disk; custody must not (#495)" --body "..."
```

The body must: link `Closes #495`, state that #500 stays open and why the two were split, name the three inverted pins, and list the issues filed in Task 5.

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the off-site duplicate chart — the practice that copies its records and keeps the copy in another building, then carries the box back after a fire.

**Steps:** paper *N* = **2** human acts to recover (fetch the box; shelve it). Architecture-forced *M* = **3** (attach the medium; run `cairn-node restore` and answer its prompts; confirm the echoed identity when provenance is not sole-enroll-signed). UI bundling target *K* = **2**. `M > N`, so this is filed as an architecture defect per house rule 7, not argued away — the extra act is the identity confirmation, which has no paper counterpart because a paper box carries no cryptographic identity to mis-assign. `K = 2` is reachable: the escrow secret and the restore invocation are one interactive ceremony, and `Provenance::Signed` on a sole-enroll medium is already unambiguous.

**Time + cognitive load:** a restore completes within **10 minutes unattended** after the operator's last keystroke, and the operator needs **one secret** (op-passphrase or recovery code) and **no knowledge of the dead node's configuration**. This slice adds **zero** steps to that ceremony — the unwrap key rides secrets the operator already holds, which is the whole reason ADR-0052's "no second ceremony" rationale was preserved. Measurement is owed by slice 2, which is the first slice whose restore recovers a readable record. **If a measurement falls outside its budget, that is the finding — file it; never adjust the budget.**

## Self-review notes

- **Spec coverage:** design §3 → Task 1; §5 keypair-at-rest → Task 3; §5 provisioning/migration → Task 4; §5 trap containment → Task 2; §5 transport → Task 6; §8 step 1–2 of the restore ordering → Task 7. Design §6, §7 and §9 (medium, registry, doors, drift guard) are **slice 2** and deliberately absent here.
- **Known deferral, declared at the surface:** slice 1's restore carries `episode_deks` without applying them, because the events they belong to do not travel until slice 2 and there is no reason to build a second custody door that slice 2 would immediately supersede. Task 7 **prints** the carried count, so the gap is visible to the operator rather than buried — the failure mode this whole design exists to correct.
- **Cross-crate trap:** Task 4 changes `ensure_unwrap_key`'s arity. Only a full-workspace run catches a missed call site, which is why Task 8 runs one explicitly.
