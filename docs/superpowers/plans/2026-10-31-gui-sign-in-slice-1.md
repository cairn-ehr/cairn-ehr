# GUI sign-in, slice 1 — the vault, per-act signing, superuser enrolment — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician signs each clinical act at a shared terminal with staff ID + passphrase against a key vault
held in the facility node's database, enrolled there by a superuser — replacing the window's `--attester-key` file.

**Architecture:** Wrapped human keys live in a node-local `vault_member` table nobody can read; the only door is a
`SECURITY DEFINER` function that checks the passphrase **in the database** (`cairn_pgx` gains an unseal function),
enforces an attempt back-off, and releases the key for ONE act over the window's node connection. The window signs
with the existing code and wipes the key (or holds it up to 15 min on a personal workstation). All human signing
goes through one `EventSigner` trait so a later in-database signer is a new implementation, not a rewrite.

**Tech Stack:** Rust (cairn-event, cairn-keystore, cairn-node, cairn_pgx/pgrx 0.18.1), PostgreSQL 18 (db/061),
Tauri 2 + plain JS (cairn-gui-tauri, no npm).

**Spec:** [`docs/superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md`](../specs/2026-10-31-gui-sign-in-and-authorship-design.md)
(slice 1 of §6). Read its §5.1–§5.4 and §9 before Task 1.

## Global Constraints

- **Interim posture (maintainer, 2026-10-31):** the key is released to the window for ONE act. The window already
  holds the node's own key (`--key`), so this is no wider than an accepted boundary. Slice 5 (autonomous terminal
  nodes) owes the in-database signer; record that obligation in the ADR (Task 8).
- **The swap seam (maintainer, 2026-10-31):** every human signature goes through `cairn_event::EventSigner`.
  No new code may take a human `&SigningKey` directly.
- **The wrapped key never leaves the database.** No grant, view or function returns `vault_member.sealed`.
- **The passphrase is never in an error, a log line or a returned string.** `cairn_vault_release` never raises on
  a wrong passphrase; window sentences are built from enums, never from a DB error's text.
- **A superuser's key is never released for a clinical act** (spec §5.1: superuser confers no clinical authorship).
  Enforced at the vault, not only in the UI.
- **Staff IDs are compared normalised:** `lower(btrim(x))`, at enrol and at release.
- **Key hold:** `--key-hold-seconds`, default **0** (shared terminal), maximum **900** (`IDLE_TIMEOUT`, state.rs:34).
- **Attempt back-off:** failures 1–3 are free; from the 4th, `next_allowed_at = now + least(2^(failures-4), 300) s`.
  A success resets it. Never a hard lock-out.
- **House rules:** TDD; no hard-coded key/seed/salt/nonce literals in tests and no `salt`/`nonce`/`iv` names on
  non-crypto values (CLAUDE.md rule 6); `db/*.sql` refusals use no `USING ERRCODE`; full workspace `cargo test`
  before PR (cross-crate call sites — cairn-sync's tests construct `AuthorParams`/`AttestParams`).
- **Deferred from the design's slice-1 row (plan-level call):** sealed tokens are NOT printed in slice 1. A sheet of
  tokens that do nothing invites someone to try one in an outage; slice 4 prints them when they work. Until then a
  locked-out last superuser is recovered with `cairn-node vault-bootstrap-superuser` at the node's console.

## Review Focus

1. **Case and whitespace in a staff ID** (`"JSmith "` vs `jsmith`) must find the same member — Task 3 test
   `release_normalises_the_staff_id`.
2. **The next nurse on a terminal with key hold > 0** types her own credential: her key replaces the held one, and a
   write with NO credential never signs as the previous person once the hold has expired — Task 5 tests
   `a_new_signer_replaces_the_held_one`, `an_expired_hold_needs_a_credential`.
3. **A wrong passphrase's sentence never contains the passphrase** and names when to try again — Task 5 test
   `refusal_sentences_never_echo_the_passphrase`.
4. **A superuser's credential on a clinical write** is refused by the vault — Task 3 test
   `a_superuser_key_is_not_released_for_a_clinical_act`.
5. **Pressing again while backed off** is not a dead end: the outcome says the seconds left and a correct passphrase
   during the wait does not count as a failure — Task 3 test `waiting_does_not_count_a_failure`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the bedside chart. Anyone on staff reads it; each entry carries its writer's signature; the
  next nurse picks up the pen with no ceremony.
- **Steps:** paper **N = 1** to sign an entry (a signature). Architecture-forced **M = 1**: the write gesture carries
  staff ID + passphrase (typed into the always-visible signer strip). UI bundling target **K = 1**; with a recent
  signer picked from the list, the ID costs one selection. Switching to the next nurse: paper 0, architecture 0.
  Whole-list sign-off stays one gesture (#288). M ≤ N.
- **Time + cognitive load:**
  - Budget: ≤ 3 s from pressing Sign to signed, credential included (pen ≈ 2 s). Argon2id at the stored params is
    ~50 ms; the rest is typing.
  - Load: no "who is logged in" state to check on a shared terminal — the strip is empty until you type.
  - Owed: the stopwatch and VoiceOver/keyboard pass, as RUNBOOK §16 (Task 8). Not measurable headlessly.

---

### Task 1: The `EventSigner` seam

**Files:**
- Create: `crates/cairn-event/src/signer.rs`
- Modify: `crates/cairn-event/src/lib.rs` (`cose_sign1_in_context` :149, `sign` :433, `sign_attestation` :691; add
  `pub mod signer; pub use signer::EventSigner;`)
- Modify: `crates/cairn-node/src/medication/sealed_submit.rs` (`AuthorParams` :29, `apply_author` :48,
  `seal_and_sign` :114), `crates/cairn-node/src/medication/attestation.rs` (`AttestParams` :20),
  `crates/cairn-node/src/chart_link.rs` (`Reviewer` :165)
- Test: `crates/cairn-event/src/signer.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  ```rust
  pub trait EventSigner: Send + Sync {
      fn verifying_key(&self) -> VerifyingKey;
      fn sign_tbs(&self, tbs: &[u8]) -> Result<Signature, EventError>;
  }
  impl EventSigner for SigningKey { /* infallible: Ok(self.sign(tbs)) */ }
  ```
  `sign(body, signer: &(impl EventSigner + ?Sized))`, `sign_attestation(addr, kid, role, signer: &(impl EventSigner + ?Sized))`,
  `seal_and_sign(body, signer: &(impl EventSigner + ?Sized))`. `AuthorParams.human_sk`, `AttestParams.human_sk`,
  `Reviewer.human_sk` become `&'a dyn EventSigner`; `apply_author` returns `(EventBody, &'a dyn EventSigner)`.
  Every existing `&SigningKey` argument still compiles (unsized coercion).

- [ ] **Step 1: Write the failing tests** in `signer.rs`: a test-only `CountingSigner { key: SigningKey, calls: AtomicUsize }`
  implementing `EventSigner` by delegation.
  - `a_wrapped_key_signs_events_byte_identically`: `sign(&body, &counting)` == `sign(&body, &key)` (bytes and
    `content_address`; Ed25519 is deterministic), and `calls == 1`.
  - `a_wrapped_key_signs_attestations_byte_identically`: same for `sign_attestation`, and `verify_attestation`
    accepts the token under `key.verifying_key()`.
  - Key from a runtime-derived seed (`std::array::from_fn(|i| (i * 7 + 3) as u8)`), never a literal.
- [ ] **Step 2: Run** `cargo test -p cairn-event signer` → FAIL (`EventSigner` not found).
- [ ] **Step 3: Implement** the trait and make `cose_sign1_in_context` call `signer.sign_tbs(tbs)` inside coset's
  `create_signature` closure (propagate the error out of the closure; `try_create_signature` if coset offers it).
  Protected-header `kid` comes from `signer.verifying_key()`.
- [ ] **Step 4: Change the three cairn-node structs and `seal_and_sign`/`apply_author`** to the `dyn` forms above.
  Do not touch any call site that passes `&SigningKey` — they must compile unchanged; if one does not, the type is
  wrong, not the call site.
- [ ] **Step 5: Run** `cargo test -p cairn-event` (PASS), then `cargo build --workspace --all-targets` and
  `cargo build --manifest-path cairn-gui/Cargo.toml --all-targets` (both compile — cairn-sync's tests and the window
  are the cross-crate callers).
- [ ] **Step 6: Commit** `refactor(sign-in): one EventSigner seam for every human signature`.

### Task 2: `cairn_vault_unseal` in `cairn_pgx`

**Files:**
- Modify: `extensions/cairn_pgx/Cargo.toml` (add `cairn-keystore = { path = "../../crates/cairn-keystore" }`),
  `extensions/cairn_pgx/src/lib.rs`, `extensions/cairn_pgx/Cargo.lock`
- Test: `crates/cairn-node/tests/vault_unseal.rs` (DB-gated)

**Interfaces:**
- Produces SQL: `cairn_vault_unseal(sealed bytea, passphrase text) RETURNS bytea` — the 32-byte seed, or NULL for a
  wrong passphrase OR an unparsable blob. `#[pg_extern(volatile, strict)]`. It never calls `pgrx::error!`.

- [ ] **Step 1: Write the failing test** `vault_unseal.rs` (opening `let Some(base) = cs() else { return };`, the
  serial guard, `connect_and_load_schema`):
  - `the_right_passphrase_returns_the_seed`: `seal::seal(&seed, "correct horse", &seal::generate_recovery_code())`,
    `to_cbor`, `SELECT cairn_vault_unseal($1, $2)` → the seed bytes.
  - `a_wrong_passphrase_returns_null_not_an_error`: → `NULL`, and the query returns `Ok`.
  - `garbage_returns_null_not_an_error`: `[0u8; 7]` → `NULL`.
- [ ] **Step 2: Run** it → FAIL (function does not exist).
- [ ] **Step 3: Implement** with `cairn_keystore::seal::{from_cbor, unseal_op}`; return the seed bytes, wiping the
  `Secret32` on drop. Refresh `extensions/cairn_pgx/Cargo.lock` (`cargo metadata` in that tree); confirm
  `cargo deny check licenses` stays green (argon2 is MIT/Apache-2.0).
- [ ] **Step 4: Reinstall the extension locally** (memory: run from inside `extensions/cairn_pgx/`, SDKROOT +
  `-Wl,-no_fixup_chains` on this Xcode; `DROP EXTENSION cairn_pgx CASCADE; CREATE EXTENSION cairn_pgx;` in each
  `cairn_test*`), then run the test → PASS. CI installs fresh (`rust.yml:350-363`), nothing else to change.
- [ ] **Step 5: Commit** `feat(sign-in): cairn_pgx checks a vault passphrase in the database`.

### Task 3: `db/061_vault.sql` — the vault floor

**Files:**
- Create: `db/061_vault.sql`, `db/tests/061_vault_test.sql` (starts with `\ir _scratch_database_guard.sql`)
- Modify: `crates/cairn-node/src/db.rs` (append to `SCHEMA` after 060),
  `crates/cairn-event/src/schema_generation.rs:45` (`SCHEMA_GENERATION = 61`)
- Test: `crates/cairn-node/tests/vault_floor.rs` (DB-gated)

**Interfaces:**
- Tables (no grants to anyone; `REVOKE ALL ... FROM PUBLIC`):
  - `vault_member(staff_id text PRIMARY KEY CHECK (staff_id = lower(btrim(staff_id)) AND staff_id <> ''),
    display_name text NOT NULL, role text NOT NULL, actor_id bytea NOT NULL, signing_key_id text NOT NULL UNIQUE,
    sealed bytea NOT NULL, hint text, contact_phone text, contact_email text,
    enrolled_by text NOT NULL /* superuser staff_id, or 'console' */, enrolled_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (contact_phone IS NOT NULL OR contact_email IS NOT NULL))`. `is_superuser` is `role = 'superuser'`, never a
    second column. One row per role-actor: a clinician-superuser has two staff IDs (#168 makes the entity first-class).
  - `vault_attempt(staff_id text PRIMARY KEY REFERENCES vault_member, failures int NOT NULL DEFAULT 0, next_allowed_at timestamptz)`
  - `vault_terminal(singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton), unlock_phc text NOT NULL)` — one
    shared staff passphrase per site (spec §9 left it to the builder; this is the call).
- Functions, all `SECURITY DEFINER SET search_path = public, pg_temp`:
  - `cairn_vault_release(p_staff_id text, p_passphrase text, p_purpose text) RETURNS TABLE(outcome text, seed bytea, signing_key_id text, retry_after_ms bigint)`;
    `p_purpose IN ('clinical','enrol')`; `outcome IN ('released','wrong','waiting','unknown','refused-purpose')`.
    Never raises for a passphrase or staff ID. `waiting` does not unseal and does not count. `refused-purpose` = a
    superuser key asked for `'clinical'`, or a non-superuser key asked for `'enrol'`. `EXECUTE` to `cairn_agent`.
  - `cairn_vault_hint(p_staff_id text) RETURNS text`; `cairn_vault_roster() RETURNS TABLE(staff_id text, display_name text, role text)`;
    `cairn_vault_terminal_phc() RETURNS text` — `EXECUTE` to `cairn_agent`.
  - `cairn_vault_enrol_address(p_staff_id text, p_signing_key_id text, p_role text) RETURNS bytea` —
    `IMMUTABLE`; `'\x1220'::bytea || sha256(convert_to('cairn-vault-enrol' || E'\n' || lower(btrim(p_staff_id)) || E'\n' || p_signing_key_id || E'\n' || p_role, 'UTF8'))`.
  - `cairn_vault_enrol(p_staff_id, p_display_name, p_role, p_pinned jsonb, p_signing_key_id, p_sealed bytea, p_hint, p_phone, p_email, p_superuser_staff_id text, p_token bytea) RETURNS bytea`
    (the new `actor_id`). Refuses unless: the superuser staff ID is a `role = 'superuser'` member whose key is in
    `actor_current`; `cairn_attestation_ok(p_token, cairn_vault_enrol_address(...), decode(<superuser kid>, 'hex'))`;
    `p_pinned ->> 'role' = p_role`. Then `enroll_actor('human', p_pinned, p_signing_key_id)` and the insert, one
    transaction. `EXECUTE` to `cairn_agent`.
  - `cairn_vault_bootstrap(p_staff_id, p_display_name, p_pinned jsonb, p_signing_key_id, p_sealed, p_hint, p_phone, p_email) RETURNS bytea` —
    role forced to `'superuser'`, `enrolled_by = 'console'`; **no grant** (owner only: the node's console).
  - `cairn_vault_set_terminal_phc(p_phc text) RETURNS void` — owner only.
- Each refusal's message says which check failed and never echoes `p_sealed` or `p_token`. Header comment explains
  for a junior why the vault is mutable custody state (like the keystore file it replaces), not clinical content.

- [ ] **Step 1: Write the failing tests** in `vault_floor.rs`, each building members through `cairn_vault_bootstrap`
  / `cairn_vault_enrol` with blobs from `seal::seal` and keys from runtime seeds:
  - `release_returns_the_enrolled_key` — seed's verifying key hex == `signing_key_id`.
  - `release_normalises_the_staff_id` — enrolled `jsmith`, released as `" JSmith "`.
  - `a_wrong_passphrase_is_an_answer_not_an_error` — `outcome = 'wrong'`, query `Ok`.
  - `the_fourth_failure_starts_a_back_off` — failures 1–3 `wrong` with `retry_after_ms IS NULL`; 4th `wrong`, then an
    immediate try is `waiting` with `retry_after_ms > 0`.
  - `waiting_does_not_count_a_failure` — during `waiting`, the RIGHT passphrase returns `waiting` and `failures` is
    unchanged; after `next_allowed_at` (set it into the past in the test) it returns `released` and resets to 0.
  - `a_superuser_key_is_not_released_for_a_clinical_act` and `a_clinician_key_cannot_enrol` — `refused-purpose`.
  - `enrol_needs_a_superuser_token_over_this_request` — a token over a DIFFERENT staff ID is refused; a token by a
    clinician is refused; the right one enrols and `actor_current` holds the new key as `kind = 'human'`.
  - `the_runtime_role_cannot_read_the_vault` — `SET ROLE cairn_agent; SELECT sealed FROM vault_member` → 42501
    (memory: the SET ROLE seam); `cairn_vault_release` still works under that role.
- [ ] **Step 2: Mirror the pure checks** in `db/tests/061_vault_test.sql` (address function determinism, the CHECKs).
- [ ] **Step 3: Run** → FAIL (no db/061).
- [ ] **Step 4: Implement** `db/061_vault.sql`; append it to `SCHEMA`; bump `SCHEMA_GENERATION` to 61. If you use
  `cairn_decode_hex_or_raise`, add the file to `hex_decode_helper.rs`'s per-file list (memory: pinned-count guards).
- [ ] **Step 5: Run** `vault_floor.rs`, the generation guards (`full_schema_list_carries_the_repo_generation`), and
  `scripts/run-db-gated-tests.sh` with `PGHOST=127.0.0.1 PGPORT=5532` (#862) → PASS.
- [ ] **Step 6: Commit** `feat(sign-in): db/061 — the vault releases a key for one act, never its wrapping`.

### Task 4: The `cairn_node::vault` library and console verbs

**Files:**
- Create: `crates/cairn-node/src/vault.rs` (add `pub mod vault;` in lib.rs)
- Modify: `crates/cairn-node/src/main.rs` (two `Cmd` variants beside `EnrollHuman` :1931, handlers beside :4588)
- Test: `crates/cairn-node/tests/vault_lib.rs` (DB-gated); unit tests in `vault.rs`

**Interfaces:**
- Consumes: Task 1's `EventSigner`, Task 3's SQL functions.
- Produces:
  ```rust
  pub fn enrol_request_address(staff_id: &str, signing_key_id: &str, role: &str) -> Vec<u8>;
  pub struct NewMember<'a> { pub staff_id: &'a str, pub display_name: &'a str, pub role: &'a str,
      pub phone: Option<&'a str>, pub email: Option<&'a str>, pub passphrase: &'a str, pub hint: Option<&'a str> }
  pub async fn enrol_member(db: &Client, new: &NewMember<'_>, superuser_staff_id: &str,
      superuser: &dyn EventSigner) -> anyhow::Result<String /* new kid */>;
  pub async fn bootstrap_superuser(db: &Client, new: &NewMember<'_>) -> anyhow::Result<String>;
  pub enum Purpose { Clinical, Enrol }
  pub enum Release { Released { key: SigningKey, kid: String }, Wrong { retry_after: Option<Duration> },
      Waiting { retry_after: Duration }, Unknown, RefusedPurpose }
  pub async fn release_for_act(db: &Client, staff_id: &str, passphrase: &str, purpose: Purpose) -> anyhow::Result<Release>;
  pub fn terminal_phc(passphrase: &str) -> anyhow::Result<String>;      // argon2 PHC string
  pub fn terminal_unlock_ok(phc: &str, passphrase: &str) -> bool;       // pure
  ```
  The superuser's token is `sign_attestation(&enrol_request_address(..), superuser_kid, "enrolled", superuser)`
  (`cairn_attestation_ok` checks the address and key, not the role string).
  The human pinned set is `build_human_pinned(role, None, Some(staff_id))` (enroll.rs:43). Key generation and sealing
  reuse `cairn_keystore::seal::seal`; the recovery code it demands is generated and discarded —
  `// shortcut: the vault re-keys by token (slice 2), so the recovery wrap is unused; drop it if slice 2 does not want it.`

- [ ] **Step 1: Write the failing tests.**
  - Unit: `the_enrol_address_matches_its_sql_twin` (DB-gated half in `vault_lib.rs`: Rust bytes ==
    `SELECT cairn_vault_enrol_address(...)` for an ID with spaces and capitals); `terminal_unlock_ok` true/false.
  - `vault_lib.rs`: `a_vault_key_signs_an_event_the_floor_accepts` — bootstrap a superuser, `enrol_member` a
    clinician, `release_for_act(.., Purpose::Clinical)`, then an `assert_medication` with
    `AuthorParams { human_sk: &key, human_kid: &kid }` through `seal_sign_submit` → submitted, and the contributor is
    graded `Attested`.
- [ ] **Step 2: Run** → FAIL (module missing).
- [ ] **Step 3: Implement** `vault.rs` (seed wiped via `Zeroizing`; `Release` maps every SQL `outcome`; an unknown
  outcome string is an `anyhow` error, never silently `Unknown`).
- [ ] **Step 4: Add the console verbs** `vault-bootstrap-superuser --staff-id --display-name [--phone] [--email]
  [--hint]` (passphrase prompted twice with `rpassword`, refused if they differ) and `vault-set-terminal-passphrase`
  (prompted twice). Both connect with `db::connect` as the owner.
- [ ] **Step 5: Run** both test files → PASS; `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 6: Commit** `feat(sign-in): cairn_node::vault — enrol, release for one act, console bootstrap`.

### Task 5: The window signs per act

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/state.rs` (drop `attester_key_path`; add `key_hold: Duration`;
  `SessionKey` gains `staff_id: String`), `src/main.rs` (`Cli`: drop `--attester-key`, add `--key-hold-seconds`),
  `src/commands.rs` (drop `unlock`; `LockState`), the 8 `*_impl` callers of `live_key`: commands.rs:252, :450;
  hold/mod.rs:135, :272; link/same_person.rs:172; link/unlink.rs:289; link/confirm.rs:80; duplicates/mod.rs:281;
  `src/funnel/window.rs:35` (`AppState::mock`)
- Create: `cairn-gui/cairn-gui-tauri/src/signer.rs` (pure decision + refusal sentences)
- Test: `src/signer_tests.rs`, plus the guard tests named in Step 5

**Interfaces:**
- Consumes: Task 4's `release_for_act`, `Release`, `Purpose`.
- Produces:
  ```rust
  #[derive(Deserialize)] pub struct Credential { pub staff_id: String, pub passphrase: Zeroizing<String> }
  pub enum HeldDecision { UseHeld, Release, NeedCredential }
  pub fn decide(held: Option<&SessionKey>, credential: Option<&Credential>, key_hold: Duration, now: Now) -> HeldDecision; // pure
  pub enum KeyRefusal { NeedCredential, Wrong { retry_after: Option<Duration> }, Waiting { retry_after: Duration },
      Unknown, NotForClinicalActs, Fault }
  impl KeyRefusal { pub fn sentence(&self) -> String }
  impl AppState { pub async fn signer_for_act(&self, credential: Option<Credential>, now: Now)
      -> Result<(SigningKey, String), KeyRefusal> }
  ```
  (`Zeroizing<String>: Deserialize` needs zeroize's `serde` feature in cairn-gui-tauri's Cargo.toml; check that
  lockfile.) `decide`: a credential always means `Release` (a new signer replaces a held one); no credential means `UseHeld`
  only if `key_hold > 0` and the held key is not expired; else `NeedCredential`. `signer_for_act` stores the released
  key as the held `SessionKey` only when `key_hold > 0`. It takes the node lock and passes errors through
  `ConnectionGuard::explain` (`node_lock_explains_tests.rs`). `KeyRefusal::Fault` carries no DB text.
  Each of the 8 commands gains `credential: Option<Credential>` and calls `signer_for_act` where it called
  `live_key`. New commands: `vault_roster`, `vault_hint(staff_id)`, `terminal_unlock(passphrase) -> bool`,
  `forget_held_key()` (the explicit Lock). `LockState { held_by: Option<String>, key_hold_secs: u64, mock: bool }`.

- [ ] **Step 1: Write the failing tests** in `signer_tests.rs`:
  - `with_no_hold_every_act_needs_a_credential` — `decide(Some(&held), None, 0s, now)` == `NeedCredential`.
  - `a_held_key_serves_the_next_act` — 900 s hold, fresh key, no credential → `UseHeld`.
  - `a_new_signer_replaces_the_held_one` — held for `jsmith`, credential for `apark` → `Release`.
  - `an_expired_hold_needs_a_credential` — reuse state.rs's `Now` test construction past `IDLE_TIMEOUT`.
  - `refusal_sentences_never_echo_the_passphrase` — every `KeyRefusal` variant's sentence; `Waiting { 12 s }` says
    "12 s"; none contains the test passphrase.
  - `key_hold_is_capped_at_the_idle_timeout` — `--key-hold-seconds 5000` is refused at parse, with the cap named.
- [ ] **Step 2: Run** `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri signer` → FAIL.
- [ ] **Step 3: Implement** `signer.rs`, `signer_for_act`, the CLI flag, the new commands (register them in
  `invoke_handler`, main.rs:108-133), and switch the 8 callers.
- [ ] **Step 4: Delete** `unlock` and `attester_key_path`; grep the tree for `attester_key` and `unlock` in comments
  and docs (README/RUNBOOK) and correct each.
- [ ] **Step 5: Update the pinned guards** to the new seam, keeping what each pins: `hold/command_tests.rs:156`
  (`the_refusals_come_before_the_key` → `"state.signer_for_act("`), `write_failure_tests.rs:73` (the locked sentence
  is now `KeyRefusal::sentence`), `link/judgement_view_tests.rs:173,180`, `commands_tests.rs:78` (`LockState` fields),
  `commands_tests.rs:352` (registered commands). A guard that only stays green by deleting its assertion is wrong.
- [ ] **Step 6: Run** the full cairn-gui gate (memory: `three-cargo-trees`; `export CAIRN_ALLOW_DB_SKIP=1` for a
  DB-free run, and once with the DB) → PASS, then `RUSTDOCFLAGS=-D warnings cargo doc` for that tree.
- [ ] **Step 7: Commit** `feat(sign-in): the window signs each act with staff ID + passphrase`.

### Task 6: The signer strip and terminal lock (webview)

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/index.html` (`section#session` :180-188), `src-ui/main.js`
  (`renderLockState` :420, `pollLock` :440, the unlock handler :450-463, `signOff` :380, `cease` :402),
  `src-ui/holds.js` (`withhold` :212, `release` :244), `src-ui/link.js` (`sendJudgement` :356, `updateLinkLock`
  :267), `src-ui/unlink.js` (:186, :222)
- Create: `src-ui/signer.js`
- Test: `src/signer_webview_tests.rs` (`source_guard::js_fn` / `js_const` pins); existing guards listed below

**Interfaces:**
- Consumes: Task 5's commands. Produces JS: `signerCredential() -> {staffId, passphrase} | null`, `clearSigner()`,
  `rememberSigner(staffId)` (in-memory last 10 on this terminal —
  `// shortcut: forgotten on restart; persist per terminal if nurses ask`).
- DOM: `form#signer` with labelled `input#staff-id` (`list="recent-signers"`, `autocomplete="off"`),
  `input#signer-passphrase` (`type=password`), `p#signer-hint` (filled from `vault_hint` on `change` of the ID),
  `button#forget-key` "Lock"; `div#terminal-lock` overlay with its own passphrase field. Ambient, never modal
  (spec: no confirmation dialogs) — the overlay is a lock screen, not a prompt.

- [ ] **Step 1: Write the failing pins** in `signer_webview_tests.rs`: each of `signOff`, `cease`, `withhold`,
  `release`, `sendJudgement`, `unlinkCompared`, `confirmCompared` passes `credential: signerCredential()` to
  `invoke` and calls `clearSigner()` in BOTH its success and its catch path (pin the exact expressions, not halves);
  `clearSigner` empties `#signer-passphrase`; the strip's inputs have `<label for>`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** `signer.js` and the wiring; replace the unlock form; `renderLockState` shows "Signed in
  for the next act: <name>" only while `held_by` is set. Every refusal goes to the existing outcome line.
- [ ] **Step 4: Update** the existing pins this moves: `link/lock_hint_tests.rs` (the 3 `pollLock` lines),
  `chart_view_webview_tests.rs:278-311`, `holds_webview_tests.rs:156,188`, `reread_webview_tests.rs:97,589`,
  `unlink_tests.rs:385`, `duplicates/webview_tests.rs:126`.
- [ ] **Step 5: Walk it headlessly** (memory: webview mock walk recipe — stub `invoke` with Rust-dumped payloads,
  Playwright; assert computed VISIBILITY, not textContent): the strip is visible, a wrong-passphrase payload shows
  its sentence, the passphrase field is empty after both a success and a refusal.
- [ ] **Step 6: Run** the cairn-gui gate → PASS. **Commit** `feat(sign-in): the signer strip and the terminal lock`.

### Task 7: Enrol staff from the window

**Files:**
- Modify: `src-ui/index.html` (a `<details id="enrol-staff">` "Enrol staff" beside the front door), `src-ui/signer.js`
- Create: `cairn-gui/cairn-gui-tauri/src/enrol.rs` (command `enrol_member`)
- Test: `src/enrol_tests.rs`, webview pins

**Interfaces:**
- Consumes: Task 4's `enrol_member`, `release_for_act(.., Purpose::Enrol)`; the signer strip supplies the superuser.
- Produces: `#[tauri::command] enrol_member(credential: Credential, member: NewMemberForm) -> Result<String, String>`
  with `NewMemberForm { staff_id, display_name, role, phone, email, passphrase: Zeroizing<String>,
  passphrase_again: Zeroizing<String>, hint }`. The superuser's key is released with `Purpose::Enrol`, used once,
  wiped — never held, whatever `key_hold` says. Roles offered: `clinician`, `nurse`, `superuser` (soft list; the
  floor accepts any role string — #168).

- [ ] **Step 1: Write the failing tests:** `mismatched_passphrases_are_refused_before_the_vault` (pure validation,
  no DB call); `a_clinician_credential_cannot_enrol` (DB-gated, `refused-purpose` sentence);
  `the_superuser_key_is_never_held` (after an enrol on a 900 s terminal, `held_by` is `None`); the hint field shows
  the shared-terminal warning (webview pin on its exact text: "Colleagues at a shared terminal can read this hint.").
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** the cairn-gui gate → PASS.
- [ ] **Step 5: Commit** `feat(sign-in): a superuser enrols staff from the window`.

### Task 8: ADR-0083, spec, runbook, handover

**Files:**
- Create: `docs/spec/decisions/0083-the-human-key-vault.md`
- Modify: `docs/spec/security.md` (§7.5 — the human vault paragraph, pointing at ADR-0083), `docs/spec/index.md`
  (version **v0.85**), `docs/spec/decisions/README.md` (index row), `mkdocs.yml` (nav entry — an un-navved ADR is
  invisible, mkdocs.yml comment), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (+ `TEMPLATE.md`) **§16**,
  `docs/developers/durable-rules.md` (a slice-1 block), `docs/HANDOVER.md`, `docs/ROADMAP.md`,
  `docs/superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md` (§6 row: sealed tokens moved to slice 4)

- [ ] **Step 1: Write ADR-0083** (Context / Decision / Consequences, house format): the vault on the facility node,
  never backed up, re-key by token after DR (ADR-0026's argument extended to humans); superuser role and what it may
  not do; the key-hold setting; **the interim release-for-one-act and the `EventSigner` seam, with slice 5 owing the
  in-database signer**; the back-off schedule; the staff-ID normalisation. Fact-check every sentence against
  db/061 and `vault.rs` before committing (memory: ADR-0077 passed two reviews with a false sentence).
- [ ] **Step 2: RUNBOOK §16:** sign ≤ 3 s from pressing Sign (credential included), next-nurse switch with 0 extra
  acts, the back-off sentence read aloud by VoiceOver, keyboard-only signing, the terminal lock. Budget outside →
  file an issue, never adjust the budget.
- [ ] **Step 3: durable-rules block:** never grant a read of `vault_member`; never make `cairn_vault_release` raise
  on a passphrase; never let a superuser key out for `'clinical'`; never take a human `&SigningKey` in new code.
- [ ] **Step 4: Run** `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict` → PASS;
  `cargo test -p cairn-node --test paper_parity_plan_section` → PASS.
- [ ] **Step 5: Commit** `docs(sign-in): ADR-0083 the human key vault; RUNBOOK §16; spec v0.85`.

### Final gate (controller, not a subagent)

Full workspace `cargo test` in the background (≈ 2 h locally — memory), the cairn-gui gate, `cargo doc -D warnings`
in both trees, `cargo deny check`, all three lockfiles `--locked`, `scripts/check_closing_keywords.py` on every
commit message — re-run after the LAST edit (memory: a gate is evidence only for the tree it ran against).
