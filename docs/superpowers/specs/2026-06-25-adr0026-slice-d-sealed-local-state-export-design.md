# ADR-0026 slice D — sealed local-state export (container shape)

- **Date:** 2026-06-25
- **ADR:** [ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) point 3
  (and the can't-retrofit, day-one requirement in the Decision preamble + point 5)
- **Status:** design approved; pre-implementation
- **Builds on:** slice A (at-rest keystore seal + recovery escrow,
  [`seal.rs`](../../../crates/cairn-node/src/seal.rs)), slice B (cold-peer backup export,
  [`backup.rs`](../../../crates/cairn-node/src/backup.rs)), slice C (restore + supersede,
  [`restore.rs`](../../../crates/cairn-node/src/restore.rs)).

## Problem

ADR-0026 point 3 requires a **sealed local-state export**: the non-event, non-signing-key
material a node holds — the data-at-rest keystore (node-default DEKs + sealed-episode DEKs),
node config, and the draft/scratchpad store — written as an encrypted bundle into the *same*
backup artifact as the cold-peer event medium. The signing key is **deliberately excluded**
(point 4: a stolen, unsealed artifact yields read access, never a signing identity). ADR-0026
names this a **day-one, can't-retrofit requirement**: the channel must exist at provisioning
or state accrued before it existed has no durability path.

### The honest scoping finding

At the current `cairn-node` (federation) tier, the things point 3 enumerates **do not exist
yet** — there is no clinical surface:

- the **signing key** is already excluded and handled by slice C (restore mints a fresh key);
- **DB state** (`node_event`, `local_node`, `sync_cursor`, `hlc_state`) is rebuilt from the
  event log on restore — not export material;
- peer relationships / display-name / address live in `node_event` (durable on the medium);
- there is **no node config file, no DEK store, no draft store** on disk today.

So the export currently has **essentially nothing to bundle**. The day-one requirement actually
bites the moment the **clinical tier** introduces DEKs/drafts — not at this tier. The decision
(taken in brainstorming) is to **build the container shape now with content later**: establish
the format, the dual-recipient secret lifecycle, the CLI verbs, the restore integration, and the
status surfacing, with the content being a versioned, additively-extensible bundle whose typed
slots are empty today. This closes point 3 *structurally* and — critically — puts the genuine
can't-retrofit piece (provisioning-time establishment of the dual-recipient local-state channel)
in place day-one.

## Key decisions (from brainstorming)

1. **Scope:** container shape now, content later (typed empty slots).
2. **Sealing secret:** dual-recipient (operational passphrase **and** recovery code), reusing the
   slice-A envelope primitives. Restore works from the recovery code alone (guaranteed-available
   paper escrow); op-pass is the convenience/runtime path.
3. **Placement:** co-located sibling file with its own magic (`CAIRNL1`) and a deterministic name
   derived from the medium path. Zero change to the tested `CAIRNB1` event-medium format.
4. **Seal lifecycle — long-lived local-state DEK (LSK):** a random 32-byte LSK is minted and
   dual-wrapped **once at `init`**; `backup` re-encrypts the bundle under the LSK using the op-pass
   (unattended); `restore` decrypts via the old recovery code. This is **ADR-0026 point-5
   compliant** (the escrow needs nothing — i.e. no recovery code — at backup time) and mirrors how
   the signing key already works (sealed once, op-pass at runtime). The init-time LSK provisioning
   is the can't-retrofit day-one piece.

### Why not the rejected alternatives

- *Reuse the signing key's DEK* — couples export confidentiality to the signing key that point 4
  excludes from backup; a stolen export+medium would leak more than "read access".
- *Seal fresh per backup under both secrets* — needs the recovery code at backup time, violating
  point 5, and is operationally heavy for an unattended cold-peer backup.
- *Seal at init/re-key only (no refresh-on-backup)* — goes silently stale once real content
  (DEKs/drafts) accrues; fails to build the actual shape.

## Architecture

```
                        provisioning (init / seal-key / establish-local-state-key)
                        ─────────────────────────────────────────────────────────
   op-pass + recovery-code ──► establish_lsk() ──► <key>.lsk  (dual-wrapped LSK, 0600)

                        steady state (backup)
                        ─────────────────────
   DB ──► read_local_state() ──► LocalState (empty today)
                                     │
   <key>.lsk + op-pass ──► seal_local_state() ──► <medium>.localstate  (CAIRNL1)

                        disaster recovery (restore)
                        ───────────────────────────
   <medium>.localstate + OLD recovery-code ──► unseal_local_state_rec() ──► LocalState
                                     │
                                apply_local_state()  (noop today)
                                     │
                          new node ──► establish_lsk() ──► fresh <key>.lsk
```

### Component 1 — `seal.rs`: share the envelope primitives (low-risk)

Leave the audited `SealedKey` / `seal` / `unseal` path **untouched** (it guards the signing key;
no regression risk taken). Expose the low-level primitives `pub(crate)` so `localstate.rs` builds
its envelope from the same crypto:

- `derive_kek`, `aead_encrypt`, `aead_decrypt`, `wrap_dek`, `try_unwrap`, `rand_bytes`,
  and the `ArgonParams` / `Wrap` types.

DRY at the primitive level, not by rewriting the seed-sealing struct. No public-API churn to
`keystore.rs`.

### Component 2 — `localstate.rs` (new module, pure + thin DB glue)

**The bundle.** A versioned struct transcribing ADR-0026 point 3's enumerated categories, all
empty today:

```rust
struct LocalState {
    version: u8,
    node_default_deks: Vec<Vec<u8>>, // empty today; node-default data-at-rest keys
    episode_deks:      Vec<Vec<u8>>, // empty today; sealed-episode DEKs (minus erased — point 6)
    config:            Option<Vec<u8>>, // None today; node config blob
    drafts:            Vec<Vec<u8>>, // empty today; draft / scratchpad store
    // The signing key is DELIBERATELY ABSENT (ADR-0026 point 4) — documented in code.
}
```

CBOR + serde defaults give **additive-only forward/backward compatibility** (principle 11):
future tiers add fields; old readers ignore unknown fields, new readers default missing ones.
Leaf types are opaque `Vec<u8>` so we reserve the *slot shape* without guessing the clinical
tier's internal schema (no speculative generality).

**LSK sealing API (Approach 1).** Built from the shared `seal.rs` primitives:

- `establish_lsk(op_pass, recovery_code) -> SealedLskWraps` — mint a random 32-byte LSK,
  dual-wrap it (op-pass KEK + recovery-code KEK via Argon2id), return the wraps + salts.
- `seal_local_state(wraps, op_pass, bundle: &[u8]) -> SealedLocalState` — unwrap the LSK with the
  op-pass, AEAD-encrypt the bundle under the LSK with a fresh nonce; emit `{ wraps, payload_nonce,
  payload_ct }` (self-contained for off-site restore).
- `unseal_local_state_rec(sealed, recovery_code) -> Option<Vec<u8>>` — restore path.
- `unseal_local_state_op(sealed, op_pass) -> Option<Vec<u8>>` — self-verify / re-export path.

`SealedLskWraps` (the `.lsk` sidecar content) holds `{ argon, salt_op, salt_rec, wrap_op,
wrap_rec }`; `SealedLocalState` (the `CAIRNL1` content) embeds those plus `{ payload_nonce,
payload_ct }`. The wraps are **stable across re-exports**; only the payload re-encrypts. `Debug`
is not derived on either (matching `SealedKey`) so a stray `{:?}` can't dump wrapped material.

**Container format + paths (pure).**

- `CAIRNL1\n` magic ++ CBOR(`SealedLocalState`); `serialize_container` / `parse_container`
  (errors, never panics, on bad magic / malformed body — mirrors `backup.rs`).
- `localstate_path_for(medium: &Path) -> PathBuf` — deterministic sibling (medium filename +
  `.localstate`).
- `lsk_sidecar_path_for(key: &Path) -> PathBuf` — `<key>.lsk`.

**DB seams (documented stubs — where the clinical tier plugs in).**

- `read_local_state(db) -> LocalState` — returns `LocalState::empty()` today; documented as the
  seam that will read DEK/draft/config tables.
- `apply_local_state(db, &LocalState)` — validates the bundle is empty / noop today; documented as
  the seam that will load DEKs into the keystore, restore config, and rehydrate drafts.

**Status surfacing (pure).**

- `describe_local_state_export(path) -> String` — `present (sealed, dual-recipient)` when the
  `CAIRNL1` sibling exists and parses; `absent — running without a local-state escrow` otherwise.

### Component 3 — CLI (`main.rs`)

- `init` → after minting the key + recovery code, call `establish_lsk` and atomically write the
  `.lsk` sidecar (0600 via `fsio`); print that the local-state escrow is established.
- `seal-key` (migration) → also establish `.lsk` (the recovery code is already in hand there).
- **new** `establish-local-state-key` → for pre-slice-D nodes: prompt op-pass + recovery code,
  write `.lsk`. Errors if one already exists (no silent overwrite).
- `backup` → after the medium write, if `.lsk` exists: `read_local_state` → `seal_local_state`
  (op-pass) → atomically write the `CAIRNL1` sibling. If `.lsk` is absent → warn ("running without
  a local-state escrow"); the event backup still succeeds.
- `restore` → if a `CAIRNL1` sibling exists alongside the medium: unseal with the **old recovery
  code** (threaded in as the restore secret), `apply_local_state` (noop today), and the new node
  establishes its **own** fresh `.lsk` at finalize. No `CAIRNL1` → restore still completes from
  events (degrades honestly).
- `status` → add the local-state-export line next to the existing backup-health line.

### No DB schema change

Content is empty today, so `read_local_state` / `apply_local_state` are pure stubs over existing
tables. No new migration (`db/010`) is introduced by this slice. This is called out explicitly so
a reviewer does not expect one.

## Error handling

Fail-safe throughout, matching slice B:

- A missing/absent `.lsk` or `CAIRNL1` **degrades honestly** — warns, never crashes; restore still
  completes from the event medium.
- Seal/unseal failures are **loud and explicit** (wrong secret / tamper → a legible error, never a
  silent wrong result; AEAD tag failure surfaces as "cannot unseal").
- `.lsk` and `CAIRNL1` writes are **atomic** (`fsio::atomic_write`, 0600) so a torn write never
  corrupts the previous good artifact.
- A stolen `CAIRNL1` + medium **without a secret** yields nothing readable (the bundle is sealed;
  it is also empty today). The signing key is never present (point 4).

## Testing (TDD — red-first)

**Pure unit (no DB):**

- LSK: `establish_lsk` → `seal_local_state` → `unseal_local_state_rec`/`_op` round-trip recovers
  the bundle; wrong secret and any tampered field → `None`.
- **Wraps stable, payload fresh:** two `seal_local_state` calls over the same wraps with different
  bundles produce identical `wrap_op`/`wrap_rec` but different `payload_ct`, and each unseals to
  its own bundle.
- `LocalState` CBOR round-trip; **additive tolerance** (a bundle serialized without a
  later-added field still deserializes with that field defaulted).
- Container `serialize`/`parse`; reject missing magic + truncated body.
- Path derivation (`localstate_path_for`, `lsk_sidecar_path_for`).
- `describe_local_state_export` present/absent.

**DB-gated integration** (self-serializing via the existing `db::test_serial_guard`):

- `backup` writes a `CAIRNL1` sibling that parses and unseals (op-pass) to an empty `LocalState`.
- Full **export → restore** round-trip on a *fresh* DB: events apply (slice C), the `CAIRNL1`
  unseal (old recovery code) yields empty state, `apply_local_state` is a clean noop, and the new
  node ends with its own `.lsk`.
- `restore` with **no** `CAIRNL1` present still completes from events (honest degradation).

## File-size discipline

Envelope primitives stay in `seal.rs` (shared, not duplicated). `localstate.rs` carries the
bundle + sealing API + container + DB seams + status helper — expected well under 500 lines. CLI
wiring is incremental additions to `main.rs`.

## Out of scope (named, not silently dropped)

- Real DEK/draft/config **content** and their DB tables — they belong to the clinical tier; the
  seams (`read_local_state` / `apply_local_state`) are where they land.
- Shamir M-of-N split, QR rendering, TPM/keyring escrow rungs (ADR-0026 point 5 upward options).
- Shred-replay into the local-state export (point 6) — N/A while there are no episode DEKs.

With this slice, **every ADR-0026 slice is closed** (A: at-rest seal; B: cold-peer export; C:
restore + supersede; D: sealed local-state export shape).
