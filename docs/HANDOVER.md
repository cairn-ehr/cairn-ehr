# HANDOVER — Cairn

**Session date:** 2026-06-25 · **Spec/ADRs:** v0.31 (+ADR-0031) · **Phase:** architecture complete; proving viability
through proof-of-concept spikes (walking skeleton, advisory-actor contract, a first federating node, Postgres-on-Android) —
no clinical implementation yet.

**This session (2026-06-25):** ADR-0026 **slice C** — node **backup restore (apply) + new-identity supersede**
([issue #50](https://github.com/cairn-ehr/cairn-ehr/issues/50)). New `cairn-node restore` rehydrates a node's signed
`node_event` history into a fresh DB under a **freshly-minted** key (the signing key is never backed up), then records
a node-level `supersede`(dead→new). The self-trusting `restore_node_event` door is **fenced empty-genesis** —
fail-closed on any already-enrolled node, so it can never bypass peer-admission on a live node — yet still enforces
signature + content-address (a tampered medium is rejected exactly as a hostile peer would be) and **never writes
`local_node`**. Schema: `db/009` (widened `op` CHECK +`supersede`, the door, `node_lineage` view) + a `supersede`
branch in `submit_node_event` (db/007); `status` now shows a `supersedes` line. Full round-trip + non-enroll-branch
tests green (8 DB-gated + pure unit). Built via the brainstorm→plan→subagent-SDD workflow (spec + plan under
`docs/superpowers/`). **Deferred** (issue #50 / ADR-0026 point 3): the sealed **local-state export** (config + drafts
+ sealed-episode DEKs); shred-replay is N/A at the node tier (no clinical bodies in `node_event` yet).

**Prior sessions (2026-06-25):** ADR-0026 **slice B** — backup-as-cold-peer **export + self-verify + health**
([PR #51](https://github.com/cairn-ehr/cairn-ehr/pull/51)): `backup`/`verify-backup` + `last_backup`; signed-event
medium, self-verifying via the signature invariant; shared `fsio` atomic-write. And Spike 0003 (Postgres on Android)
**G0–G3 PASS** + Medium write-up ([PR #47](https://github.com/cairn-ehr/cairn-ehr/pull/47),
[PR #48](https://github.com/cairn-ehr/cairn-ehr/pull/48)).

**Status of this file:** Disposable working scaffolding, **not** a source of truth. Regenerate at the end
of each session. If it ever disagrees with the canonical docs, **the canonical docs win.** The *why* lives
in the immutable ADR log; the *what* lives in the spec; this file only carries what lives *between* them —
current build state, open threads, and time-sensitive items.

---

## Read these first (the durable state)

- **`docs/spec/index.md`** — canonical architecture spec (mission prose + document map + spec version).
  One file per aspect; cross-refs like *§5.7* stay valid inside the aspect file.
- **`docs/spec/decisions/`** — the **ADR log** (the *why*). Numbered, dated, **immutable** (a reversal is a
  new superseding ADR). **Read the relevant ADR before reopening a settled question.** Index below.
- **`docs/ROADMAP.md`** — the foundation build order (wire core → in-DB floor → sync → identity →
  security → federation → blobs → native API), *below* the policy/GUI line. Disposable scaffolding like
  this file; the spec/ADRs win on any disagreement.
- **`docs/spikes/`** — build-prep records (*what we tried, on what, what we learned*). Not spec, not ADR.
- **`docs/principles/`** — mission/governance; **`GOVERNANCE.md`** + `STEWARDSHIP-OF-THE-NAME.md`.
- Root **`README.md`** — mission + founding principles (same prose as `index.md`).
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`), `/extensions` (`cairn_pgx`), `/db`.
  `poc/` is frozen historical spikes.

---

## Stale-doc cleanup — done this session

- **Status lines realigned** (spec/index.md, README.md ×2, GOVERNANCE.md ×2): were "Architecture / specification
  phase — no implementation yet" (and GOVERNANCE flatly claimed *"implementation has not started"*); now framed as
  *spec complete; proving viability through proof-of-concept spikes; no clinical implementation yet.*
- **`docs/spikes/README.md` Spike 0002 row fixed** — was *"Proposed — not yet run,"* now *"Ran ✓ — C1–C5 PASS
  (PR #27) → ADR-0029/0030."*
- **CLAUDE.md updated** — opening reframed from "no code, build system, or tests yet" to the
  proof-of-concept-spikes framing, and a new **"Coding house rules"** section enshrined (AGPL-3.0 + compatible
  deps · TDD · inline docs for a junior dev · pure reusable functions over clever complexity · fix review findings
  or file a GitHub issue).

---

## Where the build actually is (the live, in-progress state)

### First federating node — built 2026-06-21, [PR #28](https://github.com/cairn-ehr/cairn-ehr/pull/28)
First *implementation* of [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
(federation admission), scoped to **direct-pairwise trust, no clinical surface** — only the federation
machinery flows, exercising the one safety-critical seam (*verified credential → admitted peer*) E2E. **No
spec/ADR change.** Built: `cairn-node` (Ed25519 keystore, `init`/`identity`/pairing/`peers`/`unpeer`, built-in
mTLS pinned to the trust set, set-union `node_event` sync, honest `status`); `db/007` append-only `node_event`
+ `submit_node_event` door + `apply_remote_node_event` deny-all admission gate (reuses `cairn_verify` pgrx — no
new crypto). Genesis-stable `node_id` = content-address of the genesis enrollment event. Two-node E2E green on
local PG16 + `cairn_pgx`.

**Honest gaps / follow-ons declared in the node (candidate "harden the node" work):**
- ~~`status` **crashes if run before `init`**~~ **closed 2026-06-23** — `load_local_opt` (`query_opt`) +
  an `initialized` flag; `status` degrades honestly with a "run `cairn-node init`" hint
  (`tests/status.rs::status_before_init_degrades_gracefully`).
- **In-DB floor caveat** — ~~runtime should connect as a login role granted `cairn_node` (NOLOGIN)~~
  **closed 2026-06-23**: `db::provision_runtime_role` (charset-guarded against DDL injection) + a
  `provision-runtime-role` CLI subcommand create that role, and `tests/floor_enforced.rs` now **proves the
  ENFORCED path** — over a `cairn_node`-granted login role a raw `INSERT` into `node_event` is denied
  (SQLSTATE 42501), `status` reports `db_floor ENFORCED`, yet `submit_node_event` still works.
- ~~**Key-at-rest plaintext-0600**; **DR/recovery escrow a named stub** (`dr_escrow: STUBBED`)~~ **closed
  2026-06-24** (ADR-0026 **slice A**, [PR #44](https://github.com/cairn-ehr/cairn-ehr/pull/44)): the signing key is now **sealed at rest** — a random DEK seals the
  seed (XChaCha20-Poly1305), DEK **dual-wrapped** under Argon2id KEKs from an operational passphrase
  **and** a one-time **recovery code** (paper escrow, shown once at `init`). New pure `seal.rs`
  (seal/unseal/CBOR + base32 recovery code); `keystore` gained `generate_sealed`/`generate_plaintext`/
  `seal_existing` + auto-detect `load` + `key_at_rest_state`; CLI seals by default (`--insecure-plaintext`
  escape hatch) and added `seal-key` migration; daemon unseals via `CAIRN_KEY_PASSPHRASE`. `status` now
  reports `key_at_rest SEALED` + `dr_escrow recovery code set` + `recovery_escrow`. **Honest ceiling
  (documented, not engineered away): lose both the passphrase AND the recovery code → node loss.**
- ~~Genesis **HLC 0/0 placeholder**; **full-pull, no incremental watermark**~~ **closed 2026-06-23**
  ([issue #38](https://github.com/cairn-ehr/cairn-ehr/issues/38), **merged [PR #42](https://github.com/cairn-ehr/cairn-ehr/pull/42)**):
  incremental pull keyed on a monotonic local-insertion `node_event.seq` (a node always inserts newly-learned
  events with a fresh high `seq`, so the watermark is **structurally** skip-proof — decoupling it from the HLC,
  which dissolved the stated coupling), per-peer `sync_cursor` written only through an advance-only
  `checkpoint_sync_cursor` `SECURITY DEFINER` door (the runtime role keeps **zero raw DML**), with an explicit
  periodic + trust-change-triggered **full-sweep** as the correctness floor for the residual commit-order /
  rejected-then-trusted / address-remap hazards. The `0/0` HLC is now a real local clock (`hlc_state` +
  `node_hlc_tick()` + merge-forward on apply, mirroring `cairn-sync`). Acceptance test
  `sync_watermark::out_of_order_skip_is_reconciled_by_full_sweep` proves a jammed-cursor skip is reconciled by
  the sweep; the seq prefix is transport-only (signed core byte-identical, principle 12). Full node suite green
  on PG16 + `cairn_pgx`, clippy clean.
- ~~**backup-as-cold-peer** + backup-health (slice B)~~ **export half closed this session**: `backup`/`verify-backup`
  CLI + `last_backup` status line; signed-event medium, self-verifying via the existing signature invariant (tamper
  → non-zero exit); fail-safe node-local health sidecar; **verify-before-write** (the image self-verifies *before* the
  atomic rename, so a bad set never overwrites the previous good medium) plus a read-after-write tripwire gate the
  health update so it never over-claims. New `backup.rs` (pure medium format + verify + health) + shared `fsio`
  atomic-write.
- ~~**Restore (apply) + new-identity `supersede`** (slice C, [issue #50](https://github.com/cairn-ehr/cairn-ehr/issues/50))~~
  **closed this session**: `cairn-node restore` + the self-trusting `restore_node_event` door (empty-genesis fenced,
  fail-closed on a live node), node-level `supersede`(dead→new), fresh-key mint (signing key never backed up), and a
  `status` `supersedes` line. `db/009` (op-CHECK widen, the door, `node_lineage`) + a `supersede` branch in
  `submit_node_event` (db/007). Round-trip + non-enroll-branch tests green.
- Still open (remaining ADR-0026 slice): the sealed **local-state export** (config + drafts + sealed-episode DEKs,
  ADR-0026 point 3); plus Shamir M-of-N, QR, TPM/keyring escrow rungs.
- ~~atomic key-file write ([issue #45](https://github.com/cairn-ehr/cairn-ehr/issues/45)); passphrase
  `zeroize`-on-drop ([issue #46](https://github.com/cairn-ehr/cairn-ehr/issues/46))~~ **closed 2026-06-25**:
  `write_key_file` is now atomic (temp sibling → fsync → `rename` → **parent-dir fsync**, 0600 forced
  explicitly), so an interrupted `init`/`seal-key` can never leave a half-written key that boots `Corrupt`,
  the rename itself survives a power loss (not just the bytes), and a stale wide-perm `<key>.tmp` can no longer
  leak its mode onto the key; the operational passphrase and recovery code are held as `Zeroizing<String>`
  from `resolve_passphrase`/prompt through to the Argon2 call, wiped on drop (`zeroize` was already a transitive
  dep — no new crate). TDD: red-first tests for the new `tmp_sibling` helper, no-temp-litter, stale-temp clobber,
  0600 perms, stale-wide-perm-temp non-leak, and the `Zeroizing` return type. (PR #49 review: + dir fsync,
  explicit 0600, non-unix fsync.)
- Test rig: DB-gated tests need local PG + `cairn_pgx` (`cargo pgrx install` against PG16); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so plain `cargo test --workspace` is reliable.

### Spike 0002 (advisory-actor write contract) — ran 2026-06-21, C1–C5 PASS, [PR #27](https://github.com/cairn-ehr/cairn-ehr/pull/27) → ADR-0029 + ADR-0030
An external advisory agent authored an additive, un-attested, recallable advisory through the validated in-DB
door, **and the floor rejected all five hostile-agent attacks** with legible reasons. PR #27 review (the user)
caught two real floor holes the spike's own review missed — forged authorship (unbound `signer_key_id`) and a
`PUBLIC`-executable `SECURITY DEFINER` door — both fixed before merge (recorded in ADR-0030).

**Honest gap (closed 2026-06-22):** the attestation **success** path (a *valid*, correctly-bound
token accepted) was never exercised E2E — now closed by `cairn-sync attest-stdin` (the token minter),
`crates/cairn-node/tests/attestation.rs` (accept for responsibility-bearing + suppressing events; reject for
wrong-address, tampered, and non-human-attester), and `spike_0002.py` selftest (external-actor accept +
wrong-address/tamper). No `submit_event` logic changed — the accept branch already existed; this is the
coverage that was missing. **Smaller deferred items remain open** (commented in code):
`events_by_actor_epoch` resolves against `actor_current` not historical `actor_event` rows;
`actor_current` wall-clock ordering needs a monotonic tiebreaker before production; no FK on
`recall_overlay.target_event_id`; plaintext twin is skeletal.

### Dual-identifier discipline — ADR-0031, merged 2026-06-22 ([PR #34](https://github.com/cairn-ehr/cairn-ehr/pull/34); `local_ref` honesty fix merged 2026-06-24 [PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43))
New **[ADR-0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)** (canonical
identifiers + node-local surrogate keys): canonical plane (UUIDv7 + multihash) is unchanged and is the *only*
identifier on the wire/in signed bodies; the **projection plane** may intern canonical IDs to dense node-local
`bigint` surrogates as physical join keys. Leakage of a surrogate into a signed body = silent cross-node
corruption, so it is made *hard* (distinct domain type, mapping confined to floor functions, API egress always
the global ID). Landed with `db/008_surrogate_projection.sql` + the Bet B5 leakage guard. Final magnitude is
**measured on Bet B** (Pi), exactly as ADR-0001's compute bet — a "no measurable win" result narrows scope, not
fails the discipline.

**Honest gap (fixed 2026-06-24, [issue #35](https://github.com/cairn-ehr/cairn-ehr/issues/35)):** the prose
called the `local_ref` domain a "real two-way type barrier," but a PG domain over `bigint` is *not* — a
surrogate flows into any plain `bigint` with no cast/error (empirically confirmed). Corrected the wording in
`db/008`, spike 0001 §6.2, PI-RUNBOOK §6.1, and the walking-skeleton README to name the *actual* load-bearing
guarantee (signed plane typed `uuid` + `bigint ≠ uuid` + the G2 assertion) and to frame the domain honestly as
an intent-signal + one-directional guard. Rewrote **G4** in `db/tests/008_surrogate_test.sql`: it now asserts
the functions exist first (no more vacuous pass via `undefined_function`, now dropped), proves the genuine
guard (G4a `uuid`↛`local_ref`; G4b `bigint`↛`uuid` signed plane), and **characterizes the honest limit**
(G4c: `bigint` flows into `local_ref` silently). The spec body (§3.18) and immutable ADR-0031 were already
accurate (one-directional framing), so neither was touched. All G1–G6 green on PG16. **Merged 2026-06-24 ([PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43)).**

---

### Spike 0003 (Postgres on Android) — ran 2026-06-25, G0–G3 PASS, merged ([PR #47](https://github.com/cairn-ehr/cairn-ehr/pull/47) + [PR #48](https://github.com/cairn-ehr/cairn-ehr/pull/48))
Validated the **fractal-topology** invariant at the phone tier (RedMagic 11 Pro). Native PG 18.2 execs, `initdb`s,
serves SQL over TCP, and a cross-built pgrx extension loads + runs (incl. SPI) — no Termux userland, no root, no VM.
The one real blocker was `libandroid-shmem` (compile-baked Termux prefix + dead `/dev/ashmem`), fixed by a
self-contained, pinned-upstream patch. Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/) + a
Medium-style write-up. **Remaining non-load-bearing gaps:** from-source PG build and APK/`jniLibs` packaging
(not blocking — the bet is proven). No spec/ADR change.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **Clinical case-mining** — historically the highest-signal generative mode; the event-overlay + key-custody +
  actor primitives have absorbed every case so far without new architecture. Bring a real ED/hospital failure mode.
- **Dedupe transitive RustCrypto dep versions** in `Cargo.lock` ([issue #11](https://github.com/cairn-ehr/cairn-ehr/issues/11)) — supply-chain
  hygiene. **Re-verified 2026-06-25: still blocked on upstream** — the `postgres` stack pulls `digest 0.11`/`sha2 0.11`/`chacha20 0.10`
  while `chacha20poly1305 0.10.1` still depends on `chacha20 0.9` and `ed25519-dalek` on `digest 0.10`. Not fixable from our `Cargo.toml`; revisit when the ecosystem converges.
- **Harden the first federating node** — status-before-init crash, runtime-login-role/floor-ENFORCED proof,
  incremental sync watermark + genesis HLC ([issue #38](https://github.com/cairn-ehr/cairn-ehr/issues/38), PR #42),
  at-rest keystore seal + recovery escrow (ADR-0026 slice A, [PR #44](https://github.com/cairn-ehr/cairn-ehr/pull/44)),
  backup-as-cold-peer export + verify + health (ADR-0026 slice B, [PR #51](https://github.com/cairn-ehr/cairn-ehr/pull/51)),
  **and restore-apply + new-identity `supersede`** (ADR-0026 slice C, this session, [#50](https://github.com/cairn-ehr/cairn-ehr/issues/50))
  are all **closed** (see node gaps above). Next up: the sealed **local-state export** (ADR-0026 point 3) — the only
  remaining ADR-0026 slice.
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already advanced
  across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §6](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)): the
  [ADR-0001](spec/decisions/0001-fat-postgres-thin-daemon.md) projection/keystore go/no-go. Runbook + self-describing
  harness ready ([`PI-RUNBOOK.md`](../poc/walking-skeleton/PI-RUNBOOK.md)); **awaiting the Pi 5 / 16 GB / 1 TB SSD.**
  The one number that could revisit ADR-0015's *provisional* BLAKE3 blob-digest default is the ARM SHA-256-vs-BLAKE3
  result. Floor experiment = a Pi 4 / 8 GB (changes only `--label`).
- **easyGP session** — port the [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  deferred items with live easyGP schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon (validates ADR-0001 from production). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection per
  slice (the production object-store tier). The §8.2 availability + windowing/resume work already shipped.

---

## Parked (don't re-litigate without new reason)

- **Stewarding legal entity & jurisdiction** (German Stiftung/Verein, US 501(c)(3), or an umbrella) — deferred
  until momentum/funding geography is clearer.
- **Formal trademark / wordmark registration** — principle recorded (stewardship doc); legal instrument deferred.

---

## Working context (most also in CLAUDE.md)

- The user is a senior **EM physician**, GNUmed founder (early FOSS Postgres EHR), codes mostly in Python, brings
  real ED/hospital failure modes from multiple health systems. **The mission (anti-capture / anti-vendor-lock-in)
  is the tie-breaker.** Criticism is strongly encouraged — surface flaws/risks immediately.
- **Twelve founding principles** run through everything ([index.md](spec/index.md)); the first four are the lens
  for every design choice: (1) append-only + causal ordering; (2) identity is a claim — never merge/erase, always
  link/overlay; (3) paper-parity (no confirmation dialogs); (4) acknowledged uncertainty. See CLAUDE.md for the
  full set (5–12) and the §9 defect-blast-radius language-selection rule.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0 inbound=outbound,
  DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr` org; `cairn-ehr.org`+`.com`;
  PyPI/crates.io/npm `@cairn-ehr` placeholders).

---

## Decision trail — the ADR index (the *why* is in each linked ADR; do not restate it here)

**Every original §11 open architecture question is closed.** Compact index of the settled decisions; read the
ADR before reopening any of these.

| ADR | Decision (one line) | Spec home / principle |
|---|---|---|
| [0000](spec/decisions/0000-pre-adr-changelog-v0.1-v0.6.md) | Pre-ADR changelog v0.1→v0.6 | — |
| [0001](spec/decisions/0001-fat-postgres-thin-daemon.md) | Fat Postgres, thin Rust daemon | §2/§3.5/§6.1/§9.4 |
| [0002](spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md) | In-DB Rust (pgrx) escape hatch | §9.4 |
| [0003](spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md) | Bitemporal time (`t_recorded` vs `t_effective`) | §3.6/§3.7 · **principle 4** |
| [0004](spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md) | Sync scope = prefetch hint, not authority | §6.4 |
| [0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md) | Erasure = key-custody redistribution / crypto-shred | §3.8/§7.1 · **principle 9** |
| [0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md) | Replication ≠ confidentiality; the safety projection | §5.9 |
| [0007](spec/decisions/0007-authorship-and-accountability.md) | Authorship compositional, accountability separable | §3.9/§7.2 · **principle 10** |
| [0008](spec/decisions/0008-point-of-care-identity-possession-and-salvage.md) | Point-of-care identity, possession, `sign-as` salvage | §5.11/§3.10 |
| [0009](spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md) | Notification economy, salience routing, ack floor | §5.12/§3.11 |
| [0010](spec/decisions/0010-additive-vs-suppressing-classification.md) | Additive-vs-suppressing (derived, not declared) | §3.9 |
| [0011](spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) | Actor registry, version-pinning, key custody | §7.5/§3.12 |
| [0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md) | Schema evolution, two planes, legibility twin | §3.13/§6.5/§7.6 · **principle 11** |
| [0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md) | Attachments = content-addressed lazy blob tier | §3.14/§6.6 |
| [0014](spec/decisions/0014-locale-pluggable-matcher-comparators.md) | Locale-pluggable matcher comparators | §5.13/§4.1 |
| [0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md) | COSE_Sign1 + Ed25519 + SHA-256; BLAKE3 blobs (*provisional*) | §3.5/§3.14 |
| [0016](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md) | Record discovery + replicated essential tier | §6.7/§5.2 |
| [0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md) | Federation admission, sovereignty, trust anchors | §7.7 |
| [0018](spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md) | Federation revocation cascade; anchor-as-power | §7.7 |
| [0019](spec/decisions/0019-author-scoped-record-export-the-medico-legal-copy.md) | Author-scoped export (the medico-legal copy) | §7.8 |
| [0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) | Active-write, thin encounters, delete-vs-erase | §3.15 · vision §1.2 |
| [0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) | Four-layer model; node API; UI pluralism | §9.5 · **principle 12** |
| [0022](spec/decisions/0022-validated-submit-surface-the-write-path.md) | Validated `submit_event` surface (the write path) | §9.6 |
| [0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md) | Native API contract: capability + conformance | §9.7 |
| [0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md) | Hard policy = signed policy-assertion stream | §7.9 |
| [0025](spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) | ICD-11 canonical interlingua + local-terminology overlay | (terminology) |
| [0026](spec/decisions/0026-node-durability-and-disaster-recovery.md) | Node durability & disaster recovery (cold-peer backup) | §7.10 |
| [0027](spec/decisions/0027-trusted-time-anchoring.md) | Trusted-time anchoring (graded-interval `t_recorded`) | §3.17/§7.11/§6.8 |
| [0028](spec/decisions/0028-finalized-closed-contributor-role-enum.md) | Finalized closed contributor-role enum | §3.9 |
| [0029](spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md) | Skill-epoch + served-model digest as pinned actor determinants | §7.5 |
| [0030](spec/decisions/0030-advisory-actor-integration-contract.md) | Advisory-actor integration contract | §9.8 |
| [0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md) | Canonical IDs + node-local `bigint` surrogate keys (dual-identifier discipline) | §3.1/§3.2 |

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B prepared); 0002 (advisory-actor — ran, C1–C5 ✓
→ ADR-0029/0030); 0003 (Postgres on Android — **ran 2026-06-25, G0–G3 ✓**; PR #47/#48).
