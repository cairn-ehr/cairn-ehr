# #909 slice C2 — a terminal follows its node — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The owner takes one steward-signed release into the node, and every Linux (Pi, `aarch64`, Debian) terminal
fetches its window program through the pinned gateway, re-verifies it against its own pin, swaps it in, and restarts
through C1's path — a broken-into node or window can install nothing the steward did not sign.

**Architecture:** A pure verifier crate (`cairn-release`, ADR-0055's root chain + release signatures + artifact
digests) is used twice: by `cairn-node release-intake` (store in a never-synced table, serve through three read-only
`cairn_window` doors, db/086) and by a root-owned installer (`cairn-terminal-install`, a systemd path unit) on the
terminal. The window is only a courier: it fetches in 256 KiB chunks into a staging folder and reads the installer's
status file. C1's install check learns a release version so a same-schema window release also restarts.

**Tech Stack:** Rust (cairn-event `coset`/`ed25519-dalek`/`ciborium`/`sha2`, cairn-node, cairn-gui-tauri), PostgreSQL 18
SQL, systemd units, `cage` (kiosk compositor), GitHub Actions arm64 runner, plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-25-909-slice-c2-signed-release-design.md`](../specs/2026-11-25-909-slice-c2-signed-release-design.md)

Paper-parity: not clinical-surface — the slice moves signed program files from the node to its terminals; no clinical workflow gains or loses a step (the restart is C1's, RUNBOOK §40).

## Global Constraints

- **The terminal never trusts its node or its window** for a release: the installer verifies against ITS OWN pin.
  No release key in CI or on a node; the signing tool never runs on a node (design §4.2, anti-goals).
- **No default pin until the genesis ceremony:** `cairn_release::official_root()` returns `None`; every intake and
  install then refuses with `Refusal::NoChannel`. Tests mint keys at runtime (`std::array::from_fn`); never name a
  value `salt`/`nonce`/`iv` (house rule 6).
- **Signing contexts (new, never changed later):** `CTX_TRUST_ROOT = "application/cairn-trust-root+cbor"`,
  `CTX_RELEASE_MANIFEST = "application/cairn-release-manifest+cbor"`. Content address = SHA-256 of the payload alone.
- **One bundle per release, one channel, one monotonic `release_version: u64`;** artifacts typed by `kind`
  (`"window"` only today); unknown kinds are stored, digest-checked, and never served.
- **Platform today:** `os = "linux"`, `arch = "aarch64"`, `platform = "<ID>-<VERSION_ID>"` from `/etc/os-release`
  (e.g. `debian-12`). `x86_64` is not built.
- **Chunks:** the chunk door caps at **262144** bytes; the window holds the shared connection for one chunk at a
  time. A staged artifact over **64 MiB** is refused.
- **`--print-schema-generation`'s line never changes** (C1 rule); the release version comes from a NEW flag
  `--print-release-version` (one line, a `u64`), also read before clap. The build sets it from
  `CAIRN_RELEASE_VERSION` (`option_env!`); absent = `0`, which is never newer than anything.
- **Copy, verbatim** (`{who}` is `node` or `terminal`):
  - `NoChannel`: `No release channel is set up on this {who} yet, so it accepts no update. Whoever set it up must install the channel's first trust file.`
  - `ChainGap`: `This {who} last accepted trust version {pinned}; this update needs version {needs}. Take in the newer trust file, or ask the Cairn project.`
  - `RootSignaturesShort`: `Trust version {version} does not carry enough valid signatures from the version before it ({have} of {need}). Do not install it; report it to the Cairn project.`
  - `Fork`: `Two different trust files both claim version {version}. Updates are stopped on this {who} until the Cairn project resolves it; nothing that runs is affected.`
  - `ReleaseSignaturesShort`: `This update is not signed by enough of the Cairn project's release signers ({have} of {need}). Do not install it; report it to the Cairn project.`
  - `NotNewer`: `This {who} already has release {installed}; the update offered is release {offered}, which is not newer.`
  - `WrongChannel`: `This update is from channel "{offered}", but this {who} follows "{pinned}".`
  - `ArtifactMismatch`: `A file in this update is not the file its signed list names. Do not install it; fetch the update again.`
  - `Malformed`: `This update file is damaged or is not a Cairn update; fetch it again.`
  - installer `NoProgramForThisTerminal`: `This update has no program for this terminal ({os} {arch} {platform}).`
  - installer `WontStart`: `The new program does not start on this terminal; the one in place keeps running. Tell whoever looks after the node.`
  - installer `TooLarge`: `The update file is larger than any Cairn program; it was not installed.`
  - window, downloading: `This terminal is fetching its update from the node.`
  - window, installer refused: `This terminal did not install the update: {sentence}`
- Files stay under 500 lines; `main.rs` (8112) and `db.rs` (1424) gain only wiring; `cairn-event/src/lib.rs` (2133)
  gains only `pub(crate)` visibility and the two constants — new code goes in new modules.
- A new root-workspace crate refreshes `cairn-gui/Cargo.lock` and `extensions/cairn_pgx/Cargo.lock` (three-lockfile
  trap); check every new dependency's licence first (AGPL-compatible). No new crypto dependency.
- DB-free `cargo test` needs `CAIRN_ALLOW_DB_SKIP=1`; never `| tail` a gate; run every root source guard after the
  last edit (a new `cairn-gui/*/src` file is read by root guards).

## Plan-time calls (record in the ADR, Task 11)

- **The fetch rides the window's one connection**, one chunk per lock, not a second connection: a press waits behind
  at most one 256 KiB read, which is well inside the ~200 ms bound on a LAN.
- **The fetch is started from `update_state` (the 60 s poll), not a new Tauri command,** so `update.js` and the
  command registry gain nothing but the note it shows.
- **The window reads the installer's `status.json`** to learn the terminal's pinned root version and the installed
  release; a missing file means "no installer here" and the window fetches nothing.
- **`update_state` reports `waiting` when the stamp is current but `status.json` names an installed release newer than
  this program's** — it is only a hint: the restart still asks the real installed program first (C1's rule).
- **A fork freezes:** the node records an incident and its offer door returns no rows while one is open; the terminal
  installer writes `state/incident` and refuses until a human removes it.
- **The node's first trust file** comes from `release-intake --genesis-root <file>`, accepted only while the node
  holds no root; otherwise `official_root()`.

## Review Focus

1. **A press during a download** must not wait more than one chunk: the lock is dropped between chunks (Task 8 test
   holds the connection lock from a "press" between chunks and asserts the fetch waits, not the press).
2. **A half-written staging folder** (window killed mid-download, disk full) must never be installed or wedge the
   installer: `complete` is written last, and anything else in `incoming/` without a valid `complete` is cleared
   (Task 9 test).
3. **A terminal pinned at an old root version** (hand-installed months ago) must still verify: the roots door returns
   every root after its version (Task 5 + Task 9 test with a two-rotation gap).
4. **A same-schema newer window** must actually restart: `install_fit` answers `Fits` for an installed build with the
   same generation/hash and a higher release (Task 7 test), and `update_state` shows `waiting` (Task 8 test).
5. **An older hand-installed program** (lower release) must never be restarted into: `install_fit` answers `Same`
   (Task 7 test); the installer refuses `NotNewer` (Task 9 test).

---

### Task 1: Detached release signatures in `cairn-event`

**Files:**
- Create: `crates/cairn-event/src/release_sig.rs`
- Modify: `crates/cairn-event/src/lib.rs` (`mod release_sig; pub use release_sig::*;`, the two `SigningContext`
  constants beside `CTX_UNWRAP_KEY`, `cose_sign1_in_context`'s header-building helper made `pub(crate)` if needed)
- Test: `crates/cairn-event/src/release_sig.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub const CTX_TRUST_ROOT`, `pub const CTX_RELEASE_MANIFEST: SigningContext`;
  `pub fn sign_trust_root(payload: &[u8], sk: &(impl EventSigner + ?Sized)) -> Result<Vec<u8>, EventError>`,
  `pub fn sign_release_manifest(…same…)`, `pub fn verify_trust_root_sig(sig: &[u8], payload: &[u8], vk: &VerifyingKey) -> Result<(), EventError>`,
  `pub fn verify_release_manifest_sig(…same…)`, `pub fn detached_key_id(sig: &[u8]) -> Result<Vec<u8>, EventError>`.
  No public function takes a `SigningContext` parameter.

- [ ] **Step 1: Write the failing tests:** `a_trust_root_signature_round_trips`; `a_tampered_payload_fails`
  (`BadSignature`); `a_root_signature_never_verifies_as_a_manifest` (`ContextMismatch`) and the reverse;
  `an_event_signature_never_verifies_as_a_root` (sign an `EventBody` with `sign_event`, feed its payload + blob to
  `verify_trust_root_sig`: error); `a_detached_signature_carries_no_payload` (decode: `payload.is_none()`);
  `the_key_id_is_read_back`. Keys from `SigningKey::from_bytes(&std::array::from_fn(|i| i as u8 + 7))`.
- [ ] **Step 2:** `cargo test -p cairn-event release_sig` — FAIL (unresolved names).
- [ ] **Step 3: Implement** with `coset::CoseSign1Builder::create_detached_signature` /
  `CoseSign1::verify_detached_signature`, the same protected header (EdDSA, kid, content type = context) and
  `external_aad = ctx` as `cose_sign1_in_context`, and the same fail-closed header checks as `cose_verify1_parsed`
  (factor the header check into a `pub(crate) fn` both call, rather than copying it).
- [ ] **Step 4:** `cargo test -p cairn-event` — PASS (whole crate, the existing context tests included).
- [ ] **Step 5: Commit** `feat(#909): detached trust-root and release-manifest signatures (cairn-event)`.

### Task 2: `cairn-release` — the documents and their encoding

**Files:**
- Create: `crates/cairn-release/Cargo.toml` (deps: `cairn-event`, `serde`, `ciborium`, `sha2`, `thiserror` — all
  already in the tree), `crates/cairn-release/src/lib.rs`, `crates/cairn-release/src/docs.rs`,
  `crates/cairn-release/official/README.md` (says the root file lands after the ceremony)
- Modify: root `Cargo.toml` members; `Cargo.lock`, `cairn-gui/Cargo.lock`, `extensions/cairn_pgx/Cargo.lock`
- Test: `crates/cairn-release/src/docs_tests.rs`

**Interfaces:**
- Produces (all `Serialize, Deserialize, Clone, Debug, PartialEq`):
  - `Signer { kid: Vec<u8>, pubkey: [u8; 32], custody_note: String }`
  - `TrustRoot { channel_id: String, version: u64, root_signers: Vec<Signer>, root_threshold: u32, release_signers: Vec<Signer>, release_threshold: u32, prev_root_hash: Option<[u8; 32]> }`
  - `SignedRoot { payload: Vec<u8>, signatures: Vec<Vec<u8>> }` with `fn root(&self) -> Result<TrustRoot, Refusal>`, `fn hash(&self) -> [u8; 32]` (SHA-256 of `payload`)
  - `Artifact { kind: String, digest: [u8; 32], byte_len: u64, os: Option<String>, arch: Option<String>, platform: Option<String>, generation_line: Option<String> }`
  - `Manifest { channel_id: String, release_version: u64, root_version_ref: u64, artifacts: Vec<Artifact> }`
  - `ReleaseMeta { root_chain: Vec<SignedRoot>, manifest: Vec<u8>, release_signatures: Vec<Vec<u8>>, inclusion_proof: Option<Vec<u8>> }`
  - `BundleFile { meta: ReleaseMeta, artifacts: Vec<Vec<u8>> }` (artifact bytes in manifest order)
  - `fn encode<T: Serialize>(&T) -> Vec<u8>`, `fn decode<T: DeserializeOwned>(&[u8]) -> Result<T, Refusal>` using
    the canonical CBOR encoder `cairn-event` uses for `EventBody`
  - `pub fn official_root() -> Option<SignedRoot>` → `None`
  - `Refusal` enum (Task 3 fills its variants; this task adds `Malformed(String)` and `NoChannel`) with
    `fn sentence(&self, who: Who) -> String`; `enum Who { Node, Terminal }`

- [ ] **Step 1: Write the failing tests:** `every_document_round_trips`; `the_root_hash_covers_the_payload_only`
  (same payload, different signature sets → same hash); `garbage_decodes_to_malformed`;
  `there_is_no_official_root_before_the_ceremony`; `no_channel_says_who` (exact copy, both `Who`s).
- [ ] **Step 2:** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-release` — FAIL.
- [ ] **Step 3: Implement** in `docs.rs`; `lib.rs` re-exports.
- [ ] **Step 4:** PASS; then `cargo check --locked` in `cairn-gui/` and `extensions/cairn_pgx/` after refreshing their
  lockfiles (`cargo update -p cairn-release --workspace` or a plain `cargo check` without `--locked` first).
- [ ] **Step 5: Commit** `feat(#909): cairn-release documents (ADR-0055 shapes, window artifact fields)`.

### Task 3: The verifier

**Files:**
- Create: `crates/cairn-release/src/verify.rs`, `crates/cairn-release/src/refusal.rs`
- Test: `crates/cairn-release/src/verify_tests.rs`, `crates/cairn-release/src/refusal_tests.rs`

**Interfaces:**
- Consumes: Task 1's sign/verify; Task 2's documents.
- Produces:
  - `VerifiedRoot { root: TrustRoot, signed: SignedRoot, hash: [u8; 32] }`
  - `pub fn genesis(signed: &SignedRoot) -> Result<VerifiedRoot, Refusal>` — version 1, `prev_root_hash` None,
    ≥ `root_threshold` valid self-signatures
  - `pub fn advance(history: &[VerifiedRoot], chain: &[SignedRoot]) -> Result<Vec<VerifiedRoot>, Refusal>` —
    `history` ascending, non-empty; returns the new history (unchanged if the chain adds nothing)
  - `pub fn verify_release(newest: &TrustRoot, meta: &ReleaseMeta, last_installed: u64) -> Result<Manifest, Refusal>`
  - `pub fn check_artifact(manifest: &Manifest, index: usize, bytes: &[u8]) -> Result<(), Refusal>`
  - `pub fn verify_bundle(history: &[VerifiedRoot], meta: &ReleaseMeta, last_installed: u64) -> Result<(Vec<VerifiedRoot>, Manifest), Refusal>` (= `advance` then `verify_release` against the newest)
  - `Refusal` variants: `NoChannel, Malformed(String), WrongChannel{pinned, offered}, ChainGap{pinned: u64, needs: u64}, RootSignaturesShort{version, have, need}, Fork{version: u64}, ReleaseSignaturesShort{have, need}, NotNewer{installed, offered}, ArtifactMismatch{index}`

Rules `advance` must implement (ADR-0055 decisions 2, 5): an entry is accepted as version N+1 only with
`prev_root_hash == hash(N)` and ≥ N's `root_threshold` valid signatures from DISTINCT N `root_signers` (kid match,
then key verify); entries are taken in version order, duplicates of an accepted payload ignored; an entry at a version
already in `history` with a different hash that verifies against history's version − 1 is `Fork`, one that does not
verify is ignored; a gap (lowest new entry > last + 1) is `ChainGap`; two different verifiable payloads for one new
version is `Fork`; a different `channel_id` is `WrongChannel`. `verify_release`: manifest decodes, channel matches,
≥ `release_threshold` valid signatures from distinct `newest.release_signers` (never an older root's set),
`release_version > last_installed`.

- [ ] **Step 1: Write the failing tests** (a test helper builds roots and bundles from runtime keys):
  `genesis_needs_its_own_threshold`; `a_rotation_needs_the_previous_signers` (signed by the new set only → short);
  `a_two_rotation_gap_verifies_from_an_old_pin`; `a_missing_version_is_a_chain_gap` (sentence names 2 and 4);
  `two_successors_are_a_fork`; `a_differing_known_version_is_a_fork_only_if_it_verifies`; `a_regressed_chain_changes_nothing`;
  `one_signer_counted_once` (same kid twice against threshold 2 → short); `a_rotated_out_release_key_is_refused_once_seen`;
  `an_older_root_cannot_be_elected` (`root_version_ref` = 1 while newest is 2 with a different release set → only
  newest's set counts); `a_release_not_newer_is_refused`; `a_wrong_digest_or_length_is_an_artifact_mismatch`;
  `every_refusal_speaks_plainly` (each variant's sentence equals the Global Constraints copy; none contains
  "signature verification failed").
- [ ] **Step 2:** `cargo test -p cairn-release` — FAIL.
- [ ] **Step 3: Implement** — pure, no I/O, no clock.
- [ ] **Step 4:** PASS.
- [ ] **Step 5: Commit** `feat(#909): the ADR-0055 verifier — root chain, fork, release, artifacts (#257's core)`.

### Task 4: The steward's tool

**Files:**
- Create: `crates/cairn-release/src/steward.rs` (pure builders), `crates/cairn-release/src/bin/cairn-release.rs`
  (thin clap wrapper: file I/O only)
- Test: `crates/cairn-release/src/steward_tests.rs`, `crates/cairn-release/tests/cli.rs`

**Interfaces:**
- Produces: `pub fn make_genesis(channel_id, root_signers, root_threshold, release_signers, release_threshold, sign_with: &[&SigningKey]) -> SignedRoot`;
  `pub fn rotate(prev: &VerifiedRoot, next: TrustRoot, sign_with: &[&SigningKey]) -> SignedRoot`;
  `pub fn assemble(history: &[VerifiedRoot], release_version: u64, artifacts: Vec<(Artifact, Vec<u8>)>, sign_with: &[&SigningKey]) -> BundleFile`
  (fills digest and byte_len). CLI: `cairn-release genesis|rotate|keygen|bundle --key <file> …`, the bundle's window
  fields read from a CI sidecar `window.json` `{ "generation_line": "…", "release_version": N, "platform": "debian-12" }`.
  Key files use `cairn-keystore`'s `CAIRNK1` sealed format (passphrase + paper recovery code), never plain.

- [ ] **Step 1: Write the failing tests:** `an_assembled_bundle_verifies` (Task 3's `verify_bundle` +
  `check_artifact` on each); `a_rotation_built_here_advances_a_pin`; CLI: `keygen_then_genesis_then_bundle_round_trips`
  (temp dir) and `the_cli_refuses_a_sidecar_whose_release_differs`.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** `cargo test -p cairn-release` PASS.
- [ ] **Step 5: Commit** `feat(#909): the steward's signing tool (offline; never on a node)`.

### Task 5: db/086 — the release store and its three doors

**Files:**
- Create: `db/086_release_store.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs` (`SCHEMA_GENERATION = 86`; ledger entry
  `{ generation: 86, floor: 85, why: "adds the release store and its read-only doors; no window renders a new state" }`),
  `crates/cairn-node/src/db.rs` (`SCHEMA` list), `db/window_door_surface.txt` (regenerate per CONTRIBUTING's command)
- Test: `crates/cairn-node/tests/release_store_doors.rs` (DB-gated)

**Interfaces:**
- Produces tables (owner-only, not granted, never synced): `release_root(version bigint PK, channel_id text, payload bytea, signatures bytea[], root_hash bytea)`,
  `release_bundle(release_version bigint PK, channel_id text, manifest bytea, release_signatures bytea[], inclusion_proof bytea, taken_in_at timestamptz)`,
  `release_artifact(digest bytea PK, release_version bigint REFERENCES release_bundle, idx int, kind text, os text, arch text, platform text, generation_line text, byte_len bigint, content bytea)`,
  `release_incident(id bigserial PK, channel_id text, version bigint, detected_at timestamptz, detail text, resolved_at timestamptz)`.
- Doors (`SECURITY DEFINER`, `SET search_path = public, pg_temp`, `GRANT EXECUTE … TO cairn_window`):
  `cairn_release_offer(p_os text, p_arch text, p_platform text) RETURNS TABLE(release_version bigint, manifest bytea, release_signatures bytea[], idx int, digest bytea, byte_len bigint, generation_line text)` — `kind = 'window'` only, newest first, NO rows while an unresolved incident exists;
  `cairn_release_roots(p_after bigint) RETURNS TABLE(version bigint, payload bytea, signatures bytea[])` ascending;
  `cairn_release_chunk(p_digest bytea, p_offset bigint, p_len int) RETURNS bytea` — raises on `p_len > 262144` or
  `p_offset < 0`; returns `''` past the end.

- [ ] **Step 1: Write the failing tests:** `the_window_role_reads_offers_roots_and_chunks`;
  `the_window_role_cannot_read_or_write_the_tables`; `a_chunk_over_the_cap_is_refused`;
  `an_open_incident_offers_nothing`; `offers_skip_other_kinds_and_platforms`; `roots_after_returns_only_later_versions`.
  Run `window_door_surface.rs` and the floor-ledger tests too.
- [ ] **Step 2:** FAIL (no functions). **Step 3:** write db/086; bump the generation and ledger; regenerate the
  snapshot against a FRESH database (durable rule: never from a long-lived dev DB).
- [ ] **Step 4:** the new test file, `window_door_surface.rs`, `schema_replay_gate.rs`, the ledger tests — PASS.
- [ ] **Step 5: Commit** `feat(#909): db/086 — the release store and its window doors (generation 86)`.

### Task 6: The node's intake and status

**Files:**
- Create: `crates/cairn-node/src/release_intake.rs` (+ `release_intake_tests.rs` for the pure parts)
- Modify: `crates/cairn-node/Cargo.toml` (`cairn-release`), `crates/cairn-node/src/main.rs` (two `Cmd` variants and
  their dispatch only), `crates/cairn-node/src/lib.rs` (module)
- Test: `crates/cairn-node/tests/release_intake.rs` (DB-gated)

**Interfaces:**
- Consumes: Task 3 (`genesis`, `verify_bundle`, `check_artifact`), Task 5's tables.
- Produces: `cairn-node release-intake <bundle> [--genesis-root <file>]` and `cairn-node release-status`;
  `pub async fn take_in(client, bundle: &BundleFile, genesis: Option<SignedRoot>) -> Result<IntakeOutcome, IntakeError>`
  where `IntakeError::Refused(Refusal)` prints `refusal.sentence(Who::Node)` and exits 1, and a `Fork` also inserts a
  `release_incident` row. One transaction: roots, bundle and artifacts land together or not at all. Pruning keeps the
  newest two releases per `(os, arch, platform)`.

- [ ] **Step 1: Write the failing tests:** `a_signed_bundle_is_taken_in_and_offered`; `genesis_is_refused_once_a_root_is_held`;
  `no_genesis_and_no_official_root_is_no_channel`; `a_fork_records_an_incident_and_freezes_offers`;
  `a_bad_artifact_leaves_nothing_behind`; `the_same_release_twice_is_not_newer`; `old_releases_are_pruned_to_two`;
  `status_names_channel_root_release_and_incident`.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** PASS, plus the root source guards (DB-free) and
  `db_errors_stay_legible.rs` (a new `LocalDbFault`-wrapped read raises its per-file count: register it).
- [ ] **Step 5: Commit** `feat(#909): cairn-node release-intake and release-status`.

### Task 7: The window knows its release

**Files:**
- Modify: `crates/cairn-node/src/window_floor.rs` + `window_floor_tests.rs` (`install_fit`),
  `cairn-gui/cairn-gui-tauri/src/update.rs` (`RELEASE_FLAG`, `release_version()`, `installed_fit` reads both lines),
  `cairn-gui/cairn-gui-tauri/src/main.rs` (answer `--print-release-version` before clap, beside the generation flag),
  `cairn-gui/cairn-gui-tauri/src/update_tests.rs`

**Interfaces:**
- Produces: `pub struct InstalledBuild { pub generation: i32, pub hash: [u8; 32], pub release: u64 }`;
  `install_fit(installed: Option<InstalledBuild>, running: InstalledBuild, stamp) -> InstallFit` — `Same` if `None`,
  identical, or `installed.release < running.release`; otherwise `window_verdict(stamp, installed.generation)` as now.
  `pub const RELEASE_FLAG: &str = "--print-release-version"`; `pub fn release_version() -> u64` (from
  `option_env!("CAIRN_RELEASE_VERSION")`, unparsable/absent = 0). A program that cannot answer the release flag reads
  as release 0.

- [ ] **Step 1: Write the failing tests:** `a_same_schema_higher_release_fits`; `a_lower_release_is_same`;
  `the_release_flag_is_read_before_clap`; `the_release_line_round_trips`; and, `#[cfg(target_os = "linux")]`,
  `a_program_renamed_over_while_running_is_read_at_its_path` (copy `/bin/sleep` to a temp path, spawn it, write a new
  file and `rename` it over, assert `exe_path(read_link("/proc/<pid>/exe"))` is the original path and the file there
  is the new one) — this is design §4.5's "run early on Linux" check; CI's Linux job runs it.
- [ ] **Step 2:** FAIL. **Step 3:** implement; existing `install_fit` callers pass `InstalledBuild`.
- [ ] **Step 4:** `cargo test -p cairn-node window_floor`, then in `cairn-gui/`: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri update` — PASS.
- [ ] **Step 5: Commit** `feat(#909): the window reports and compares its release version`.

### Task 8: The window as courier

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/release_fetch.rs` (+ `release_fetch_tests.rs`)
- Modify: `cairn-gui/cairn-gui-tauri/Cargo.toml` (`cairn-release` for the types only), `main.rs` (`--release-dir <dir>`
  flag into `AppState`), `update.rs` (`update_state_impl` starts a fetch when due; `UpdateView.note: Option<String>`),
  `src-ui/update.js` (shows `note` on the update line)
- Test: `release_fetch_tests.rs`, `update_webview_tests.rs`, a DB-gated `release_fetch_db_tests.rs`

**Interfaces:**
- Consumes: Task 5's doors, Task 7's `release_version()`, the installer's `state/status.json` (Task 9 defines it:
  `{ "state": "idle"|"installed"|"refused", "installed_release": u64, "pinned_root_version": u64, "sentence": string|null }`).
- Produces: `pub fn due(status: Option<&InstallerStatus>, offers: &[Offer], target: &Target) -> Option<Offer>` (pure:
  newest offer whose release > `installed_release` and > `release_version()`, whose `generation_line` fits the stamp via
  `install_fit`, not already staged); `pub async fn fetch(state: &AppState, offer: Offer, dir: &Path)` (writes
  `incoming/meta.cbor` = `ReleaseMeta` with `cairn_release_roots(pinned_root_version)`, `incoming/artifact.bin` by
  262144-byte chunks, each chunk under its own `state.db` lock, then `incoming/complete` containing the release number;
  at most one fetch at a time); `Target` from `std::env::consts` and `/etc/os-release`.

- [ ] **Step 1: Write the failing tests:** `due_picks_the_newest_fitting_newer_offer`; `due_skips_a_release_already_installed`;
  `no_status_file_fetches_nothing`; `no_release_dir_fetches_nothing`; `mock_never_fetches`;
  `complete_is_written_last` (kill after the artifact: no `complete`); `a_press_waits_at_most_one_chunk` (DB-gated:
  a task takes `state.db` between chunks; the fetch resumes after it); `a_newer_installed_release_shows_waiting`;
  `a_refused_install_shows_its_sentence` (webview: the copy from Global Constraints, visible — assert computed display).
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** the cairn-gui gate (`cargo test`, `clippy -D warnings`,
  `cargo doc` with `RUSTDOCFLAGS=-D warnings`) and its DB-gated tests locally (#905: CI does not run them).
- [ ] **Step 5: Commit** `feat(#909): the window fetches a fitting release into its staging folder`.

### Task 9: The terminal installer

**Files:**
- Create: `crates/cairn-terminal-install/` (`Cargo.toml`; `src/main.rs` thin; `src/decide.rs` pure; `src/swap.rs`;
  `src/status.rs`; tests beside each); root `Cargo.toml` members + the three lockfiles

**Interfaces:**
- Consumes: Task 3 (`genesis`, `advance`, `verify_release`, `check_artifact`), Task 7's two print flags.
- Produces: `cairn-terminal-install --staging <dir> --state <dir> --program <path> --run-as <user> [--genesis-root <file>]`
  (defaults `/var/lib/cairn-terminal/incoming`, `/var/lib/cairn-terminal/state`, `/opt/cairn/bin/cairn-window`,
  `cairn-window`, `/etc/cairn/channel-root.cbor`); state files `state/roots.cbor` (`Vec<SignedRoot>`),
  `state/installed_release`, `state/incident`, `state/status.json` (mode 0644, Task 8's shape).
  `pub fn decide(history, meta, artifact_len, last_installed, this: &Target, incident: bool) -> Result<(Vec<VerifiedRoot>, Manifest, usize), Refusal>`.
  Order: incident → refuse; size > 64 MiB → `TooLarge`; `verify_bundle`; pick the `window` artifact matching `this`
  (else `NoProgramForThisTerminal`); `check_artifact`; run the new file as `--run-as` (uid/gid via
  `std::os::unix::process::CommandExt`) with both print flags, 5 s timeout each, and require the manifest's
  `generation_line` and `release_version` (else `WontStart`); write the new program with `cairn_keystore::fsio::atomic_write` to `<program>.new` (mode 0755; it fsyncs),
  hard-link the current program to `<program>.prev`, `rename` over, `fsync` the directory; record state; clear staging.
  Every refusal clears staging, writes `status.json` with the sentence (`Who::Terminal`), and changes nothing else.

- [ ] **Step 1: Write the failing tests:** `decide` cases mirroring Task 3 plus `an_incident_refuses_everything`,
  `a_platform_mismatch_names_this_terminal`, `an_oversized_file_is_too_large`; swap (temp dirs, a tiny shell script
  as the "program" that prints both lines): `a_verified_release_replaces_the_program_and_keeps_prev`,
  `a_program_that_will_not_start_is_never_swapped_in`, `a_half_staged_folder_is_cleared`, `a_lower_release_is_not_newer`,
  `a_terminal_pinned_two_rotations_back_installs`, `status_json_matches_the_windows_reader` (Task 8's parser reads it).
- [ ] **Step 2:** FAIL. **Step 3:** implement (Unix-only crate: `#![cfg(unix)]` with a compile error elsewhere).
- [ ] **Step 4:** `cargo test -p cairn-terminal-install` PASS; root source guards PASS.
- [ ] **Step 5: Commit** `feat(#909): the terminal installer — verifies against its own pin, swaps atomically`.

### Task 10: Packaging and the CI release build

**Files:**
- Create: `packaging/terminal/cairn-terminal-install.path`, `.service`, `cairn-window-kiosk.service` (runs `cage --
  /opt/cairn/bin/cairn-window --terminal-dir … --release-dir /var/lib/cairn-terminal`), `packaging/terminal/install.sh`
  (first hand install: user `cairn-window`, the layout and modes, `cage` from apt, the trust file to `/etc/cairn/`,
  prints the trust file's SHA-256 to compare with the second channel), `packaging/terminal/README.md`;
  `.github/workflows/release-build.yml` (manual + tag trigger, `runs-on: ubuntu-24.04-arm`, `container: debian:<the
  release Raspberry Pi OS Lite 64-bit is based on — check and pin it>`; builds `cairn-gui-tauri` and
  `cairn-terminal-install` with `CAIRN_RELEASE_VERSION`; writes `window.json` by running the built window's two print
  flags; uploads artifacts + SHA-256s; NO signing step)
- Test: `crates/cairn-terminal-install/tests/packaging.rs` (source checks: the path unit watches `incoming/complete`;
  the service runs as root; the kiosk unit never runs as root; `install.sh` sets `incoming` to group `cairn-window`
  mode 2770 and `state` to root 0755; the workflow holds no `secrets.` reference)

- [ ] **Step 1:** licence check `cage` (MIT expected) and record it in the README; write the failing packaging test.
- [ ] **Step 2:** FAIL. **Step 3:** write the units, script and workflow. **Step 4:** PASS; `shellcheck install.sh` if present.
- [ ] **Step 5: Commit** `feat(#909): terminal packaging and the unsigned aarch64 release build`.

### Task 11: ADR, spec, RUNBOOK, durable rules, tracking docs

**Files:**
- Create: `docs/spec/decisions/0112-a-terminal-follows-its-node.md` (decisions D1–D6, the plan-time calls, the window
  artifact's manifest fields, the terminal's own pin, offline steward signing; cites ADR-0055, ADR-0110, ADR-0111;
  amends nothing by edit)
- Modify: `docs/spec/security.md` §7.6 (one paragraph: what is built), `docs/spec/index.md` (version **v0.114**, ADR
  map), `docs/spec/decisions/README.md` or index of ADRs, `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (new **§41**, after §40: intake time; LAN download time on
  a Pi; the longest press delay during a download ≤ ~200 ms; swap time; intake → "update ready" end to end; the genesis
  ceremony steps), `docs/developers/durable-rules.md` (a `⇒ #909 SLICE C2` block: never trust the node/window; no
  context parameter on the public sign/verify; one chunk per lock; `complete` last; the print lines never change;
  no signing in CI — each with its pin and tempting wrong fix), the design's status line, `docs/HANDOVER.md`,
  `docs/ROADMAP.md`; file issues for what stays open (the genesis ceremony; #258's log check; a node restored to an
  older release)
- [ ] **Step 1:** write the docs; reflow; `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`.
- [ ] **Step 2:** `scripts/check_closing_keywords.py` on the commit message; the full gates in CI's order after the
  last edit.
- [ ] **Step 3: Commit** `docs(#909): ADR-0112, spec v0.114, RUNBOOK §41, durable rules for slice C2`.
