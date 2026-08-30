# Design — the shared keystore crate: cairn-sync loads the node's custody key, never re-derives it

- **Date:** 2026-08-28
- **Closes:** [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503) (cairn-sync derives its unwrap
  secret instead of loading the node's provisioned one)
- **Produces:** a new workspace member `crates/cairn-keystore`; a new `crates/cairn-sync/src/unwrap_key.rs`
  module; **no ADR, no spec bump, no migration** — this implements
  [ADR-0066](../../spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md), it does not
  decide anything ADR-0066 left open
- **Operationally:** federated sync is **inoperable on `main`** until this lands — see §1

## 1. What is actually broken

[ADR-0066](../../spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md) broke the coupling
between a node's signing identity and its X25519 unwrap (custody) key: `cairn-node` now provisions an
**independent** unwrap keypair into a sealed `<key>.unwrap` file and registers its public half in
`node_unwrap_key`.

`cairn-sync` was not converted. It still HKDF-derives its unwrap secret from its signing seed at six
production sites. Against any node provisioned by today's `cairn-node init`, the derived key and the
registered key **disagree by construction**, so `cairn-sync pull` / `serve` / `run` refuse to start.

That refusal is correct and deliberate — the alternative is a daemon silently wrapping DEKs to a key the
node cannot open, which is indistinguishable from "this peer has no custody to offer" (the #495 failure
shape, one layer up). But it means **no node built on this branch can federate.** Single-node work,
`backup` and `restore` are unaffected.

DR slice 1 shipped the fence (`unwrap_key_matches` + `assert_unwrap_key_registered`) as an interim
mitigation and filed #503 for the real fix.

## 2. Why cairn-sync cannot simply read the file today

`cairn-sync` has no production dependency on `cairn-node`, and both halves of what it would need live
there: the `CAIRNK1` sealed-bundle format (`cairn-node/src/seal.rs`) and the loader
(`cairn-node/src/keystore.rs`), along with the `argon2` dependency. A production dependency on
`cairn-node` — an application crate carrying clap, rustls, rcgen, tokio-postgres and every orchestrator —
is the wrong direction. Hence the extraction.

### A contradiction found while reading, and deliberately left open

`cairn-node` and `cairn-sync` **do not share a key-file format**:

- `cairn-node --key` (default `node.key`) is raw 32 bytes or a sealed `CAIRNK1` CBOR bundle, unsealed with
  `CAIRN_KEY_PASSPHRASE` or a recovery code.
- `cairn-sync --key` is a **hex-encoded text** seed, and `main.rs:798` explicitly refuses a binary file:
  *"it looks binary — a sealed cairn-node key? … Point `--key` at this daemon's own key file."*

The divergence fence added by DR slice 1, 450 lines above it, tells the operator the opposite: *"point
`--key` at the same key this node was provisioned with"* (`main.rs:345`). **Following that instruction hits
the other message's refusal.** A real single-node deployment cannot currently point both binaries at one
key file; the A→B test rig only works because it hand-writes the same seed twice in two formats
(`clinical_pull.rs:252-272` documents this in full).

This is a genuine operator trap, and it is **out of scope here** (§8): unifying the signing-key format
touches every `cairn-sync` verb that loads a key plus the test rig's `write_key_file` and its
load-or-CREATE semantics, and #503 does not need it. Filed, not fixed — house rule 5.

## 3. The crate

**`crates/cairn-keystore`** — the at-rest key-file layer. Three files move **verbatim**; this slice
changes no sealing, loading or key-derivation logic.

| Moved from | To | Why it must go |
|---|---|---|
| `cairn-node/src/seal.rs` (592 lines) | `cairn_keystore::seal` | The `CAIRNK1` sealed-bundle format, the argon2 KEK, Crockford recovery codes |
| `cairn-node/src/keystore.rs` (645 lines) | `cairn_keystore::keystore` | Signing- and unwrap-key file load/write, `unwrap_key_path_for`, `KeyAtRest` |
| `cairn-node/src/fsio.rs` (~110 lines) | `cairn_keystore::fsio` | `atomic_write` / `sibling_with_suffix` / `tmp_sibling` — `keystore` cannot move without it |

Dependencies: `cairn-event`, `argon2`, `chacha20poly1305`, `ciborium`, `getrandom`, `serde`, `zeroize`,
`thiserror`. **Every one is already a vetted cairn-node dependency**, so house rule 1 (AGPL-3.0
compatibility checked *before* adding) is satisfied by construction — no new licence surface enters the
project. `rpassword` stays behind in `cairn-node`: it is the interactive CLI prompt, not the format.

`publish = false`, `[lints] workspace = true`, mirroring the other members. No dependency cycle:
`cairn-event` depends on neither crate.

### Why a *keystore* crate owns generic file IO

`fsio` is not key-specific, and `backup.rs` and `main.rs` use it too. It moves anyway because
`keystore::generate_sealed` and friends cannot function without `atomic_write`, and splitting a 110-line
module across two crates to avoid a slightly broad crate name would be worse. It stays a *public* module
so cairn-node's other two callers reach it unchanged.

## 4. cairn-node keeps its call sites

There are **221** `keystore::` / `seal::` / `fsio::` references across ~30 cairn-node source and test
files. Rewriting all of them would bury the actual behavioural change in rename noise. Instead
`src/lib.rs` gains:

```rust
pub use cairn_keystore::{fsio, keystore, seal};
```

and the three source files are deleted. `crate::keystore::load(...)` keeps resolving everywhere,
unchanged.

The re-export is not a compatibility shim to be removed later — it is the honest statement that
`cairn-node` still *offers* these modules, now implemented elsewhere. Three lines, and a reviewer reading
`crate::keystore::` in any of those 30 files finds the definition in one hop.

## 5. cairn-sync: resolve once, thread it down

Today `cairn-sync` derives its unwrap secret at six production sites **independently**. Two of them are
the real users (`do_pull`, the serve path); three are the startup fences; one is a microbenchmark. Nothing
makes the fence's derivation and the point-of-use derivation agree — they are equal today only because
they are the same pure function of the same seed.

The fix is one resolution at startup, threaded into the two places that consume it.

### 5.1 The new module

**`crates/cairn-sync/src/unwrap_key.rs`.** `main.rs` is 11,697 lines with no modules; this slice does not
refactor it, but it does not add to it either. The module holds one **pure** decision function plus a thin
DB-touching wrapper — the pure half is where the whole safety argument lives, and it is testable with no
database and no filesystem.

```
resolve(loaded: LoadOutcome, derived: &[u8; 32], registered: Option<&[u8; 32]>) -> Resolution
```

### 5.2 The decision table

| Situation | Outcome |
|---|---|
| `<key>.unwrap` loads, public half **matches** the registered key | `Loaded` — start |
| `<key>.unwrap` loads, public half **diverges** | **Refuse** — today's fence, message unchanged |
| File **absent**, derived key **==** registered | `DerivedFallback` — start, **warn on every startup** |
| File **absent**, derived key **≠** registered | **Refuse** — a restored node; the #495 shape exactly |
| File **present but unreadable / corrupt** | **Refuse** — never falls back |
| No row in `node_unwrap_key` at all | Start without custody — today's `None` case, unchanged |
| Row present but not 32 bytes | **Refuse** — today's behaviour, unchanged |

Three rows carry the reasoning that is not obvious:

**The fallback cannot be silently wrong.** A derived key that *equals the registered key* is provably the
key those `event_dek` rows are wrapped to — using it is correct, not a reintroduction of the #495
coupling. The coupling #495 named was a *restored* node deriving a **fresh** key; that node's derived key
differs from the registered one and lands on the refuse row. The fallback is admissible precisely because
it verifies against the database before trusting itself.

**Absent falls back; unreadable refuses.** #502 item 3 found that reporting a present-but-unusable `.lsk`
sidecar as "absent" sent the operator to a command that then refused. The same distinction binds here for
a sharper reason: on an adopted node a corrupt `.unwrap` file would be *masked* by a successful derive, and
that file is the only vehicle carrying custody off the machine (`localstate`'s sealed export reads it). A
node could then back up nightly, sync happily, and discover at restore that its custody key was bit-rotted
for months.

**The fallback is loud, every time.** Not a one-off notice: a warning naming the missing path and
`cairn-node establish-unwrap-key` on every startup. A silent fallback is how a deleted key file stays
invisible until a restore needs it. Note HANDOVER's trap 4 before running that command on a *restored*
node — the warning text must not push an operator into it blind.

### 5.3 Where the key comes from

The `<key>.unwrap` sibling of `--key` (via `cairn_keystore::keystore::unwrap_key_path_for`, the same rule
cairn-node uses), overridden by `--unwrap-key PATH`. Sealed files unseal via `CAIRN_KEY_PASSPHRASE`,
exactly as cairn-node does — `KeystoreError::Sealed` is already a distinct variant for precisely this.

### 5.4 The six sites

| Site | Change |
|---|---|
| `main.rs:4316` (`cmd_pull` fence) | resolve once, keep the result |
| `main.rs:4645` (`cmd_run` fence) | resolve once, keep the result |
| `main.rs:5564` (`serve` CLI arm fence) | resolve once, keep the result |
| `main.rs:3159` (`do_pull`) | **receives** the resolved secret instead of deriving |
| `main.rs:5184` (serve path) | **receives** the resolved secret instead of deriving |
| `main.rs:2849` (`cmd_bench_seal`) | `generate_unwrap_secret()` — a throwaway microbench recipient; identical measurement, one less coupling |

## 6. The guard, and what #503 can honestly claim

`crates/cairn-node/tests/unwrap_secret_is_not_derived.rs` sweeps every shipping tree for calls to
`derive_unwrap_secret` against an `ALLOWED` list, and **asserts every entry is still live** — a dead entry
fails the guard, so the list cannot quietly widen. Two entries change:

1. `crates/cairn-node/src/keystore.rs` → **`crates/cairn-keystore/src/keystore.rs`**. Path only; the
   ADR-0066 adoption migration (`adopt_derived_unwrap_secret`) moves with the file.
2. `crates/cairn-sync/src/main.rs` **stays, with a rewritten reason.** Its current text — *"cairn-sync
   cannot read cairn-node's keystore (no production dependency … tracked by #503, which is the real
   fix)"* — becomes **false** the moment this lands. The replacement names the only remaining reason: the
   pre-ADR-0066 adoption fallback of §5.2.

The new crate sits under `crates/`, so `sources::PRODUCTION_TREES` sweeps it automatically. Its moved unit
tests are `#[cfg(test)]`-gated and stay invisible to the offender scan.

**Therefore #503 closes with one derive site still live in `cairn-sync`, by design**, and its acceptance
checkbox *"cairn-sync no longer calls `derive_unwrap_secret`"* is not tickable. This slice closes #503 on
the extraction — which is what the issue's own title and recommendation ask for — and **files a follow-up**
to retire the fallback once no pre-ADR-0066 node can exist. Redefining an issue's stated criteria to match
what was built is the quotable-half-truth failure mode DR slice 1 exists to correct; the follow-up is the
honest form.

## 7. Testing

TDD throughout — failing test first, per house rule 2 and §9 (key custody is safety-critical: a defect
here silently wraps DEKs to a key the node cannot open).

1. **The pure decision table** (`unwrap_key.rs`, no DB, no filesystem): one test per row of §5.2, plus the
   anti-vacuity pin that the happy path passes *first*. This is where the safety argument is proved.
2. **The loader seam** (`TempDir`): a real sealed `.unwrap` written by `cairn_keystore`, loaded by
   `cairn-sync` — the cross-crate link that no test carries today. DR slice 1's lesson 4 applies directly:
   *where no test carries the value across the disk, the one link that matters is proven by nothing.*
   A populated round-trip, not a shape assertion.
3. **The three startup arms**: refuse, fall back with a warning, and start cleanly.
4. **The moved code stays green under `-p cairn-keystore`** — its own unit tests travel with it and are
   the extraction's regression proof.
5. House rule 6: every key/seed fixture derived at runtime, never a literal.

The gate is `scripts/run-db-gated-tests.sh` — the only one that catches this repo's three demonstrated
hiding modes (fail-fast, a piped exit status, a cross-crate suite `-p <crate>` never builds). The
cross-crate mode matters here specifically: `cairn-sync/tests/clinical_pull.rs` exercises both binaries'
custody path and `-p cairn-node` never builds it.

## 8. Scope

**In:** the crate extraction, cairn-node's re-export, cairn-sync's resolution module and its six sites, the
guard's two `ALLOWED` entries, the tests above.

**Out, filed not fixed:**

- The hex-vs-sealed **signing**-key format split and its two contradictory error messages (§2).
- Retiring the derived fallback (§6).
- Any refactor of `cairn-sync/src/main.rs` beyond not adding to it.

**Paper-parity (§1.2):** *not clinical-surface — this is the node's at-rest key-file layer and a daemon's
startup key resolution; it adds and changes no clinician-facing act.*
