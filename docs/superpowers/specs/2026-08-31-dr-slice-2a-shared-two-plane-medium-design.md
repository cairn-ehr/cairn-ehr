# Design — DR slice 2a: the shared, two-plane, append-only backup medium

- **Date:** 2026-08-31
- **Part of:** DR slice 2, the programme that closes
  [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) (*the backup medium carries no clinical
  event*). **This slice closes nothing.** It is the format every later piece reads and writes — see §2
  for the decomposition and §3 for what stays broken when it merges.
- **Produces:** a new workspace member `crates/cairn-medium`; a new `CAIRNB3` container revision; a
  `NIL_PATIENT` move into `cairn-event`. **No ADR, no spec bump, no migration, no DB.**
- **Supersedes nothing yet.** The ADR that supersedes
  [ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) decision 2's
  implementation wording is slice 2e, and merges *with* the first piece that makes it true.

---

## 1. The defect this programme exists for

ADR-0026 decision 1 promises that on total hardware loss of a solo node, *"the **clinical event log
survives**"*. Decision 2 says *"Clinical events back up as a cold peer … a configuration of the existing
sync daemon whose peer is a local, always-attached, encrypted volume."*

What is built is neither. `crates/cairn-node/src/backup.rs::read_event_set` is

```rust
.query("SELECT signed_bytes FROM node_event ORDER BY seq", &[])
```

`node_event` is the **federation plane**: enrolments, pairings, supersedes. No `event_log` row travels —
not clinical, not demographic, not identity, not registration, not erasure. A solo clinic backs up
nightly, `verify-backup` passes, `backup-status.json` records a true count of what the medium actually
holds, the disk dies, and `restore` recovers **who it peered with and zero patients**.

Every surface is honest; the composite is a precise untruth. That is #500.

Its sibling #495 — a restored node could not open inherited custody — is **closed** by
[ADR-0066](../../spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md): the node's X25519
unwrap secret is now an independent keypair that rides the `CAIRNL1` local-state export and is *adopted*
by `restore`. So the key path is finished and the byte path is not: **a restored node today has a working
key and nothing to open with it.**

---

## 2. The programme, and the three decisions already taken

The maintainer chose the faithful reading of ADR-0026 decision 2 over the cheaper one: **the medium
becomes a genuine sync peer**, not a widened bespoke exporter. Three decisions follow from that and are
settled (2026-08-31):

1. **Mechanism — a real sync peer.** The medium is addressed through the same request/response protocol a
   network peer is, so *"backup is a configuration of the sync daemon"* becomes true rather than aspirational.
2. **Orchestration — `cairn-node backup` stays the single operator command.** It writes its own node plane
   and drives the clinical capture. One command, one artifact, no added human act: paper-parity holds and
   [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512)'s `M > N` finding is not made worse.
3. **The actor registry rides the sealed export.** `actor_event` (db/004) has no `signed_bytes` and
   replicates nowhere, while every clinical apply door gates on `actor_current`
   (*"signer % is not an enrolled, non-revoked actor"*). Without the registry a restored node refuses its
   own history. It travels in `CAIRNL1` beside the unwrap secret and the DEKs — ADR-0026 decision 3's own
   category, *non-event trust material that cannot ride the cold peer*. **Named caveat for slice 2e's ADR:**
   those rows arrive authenticated by the container's AEAD, **not by per-row signatures**, so they are the
   one part of a restore that is not verify-on-apply.

### The five pieces

| | Piece | What lands |
|---|---|---|
| **2a** | *this spec* — shared two-plane medium format | The crate, `CAIRNB3`, the clinical section, the attestation chain. Pure, zero DB. |
| **2b** | Transport seam + paged pull | `request(peer, req)` behind a trait (`TcpTransport` + `MediumTransport`); a batch limit on `EventsAfterSeq`. |
| **2c** | Backup captures both planes | `cairn-node backup` drives the clinical capture from the medium's own watermark; shred-aware; health reports *scope*. |
| **2d** | Restore brings the record back | Registry rides `CAIRNL1`; `apply_local_state` installs it; restore pulls the medium through `apply_remote_event` **unchanged**. |
| **#511** | Custody newtypes | `Secret32`/`PublicKey32` across the custody plane. **Must land before 2c** — see below. |
| **2e** | The ADR | Supersedes ADR-0026 decision 2's implementation wording; records the two planes, the export-borne registry and its caveat, and what is *still* not true. |

### Why #511 is sequenced into the middle of this programme

Every key in the custody plane is a bare `[u8; 32]` — the X25519 secret half, the X25519 public half, the
Ed25519 signing seed and a DEK are all the same type — so installing a *public* key as this node's
*secret* custody key compiles today ([#511](https://github.com/cairn-ehr/cairn-ehr/issues/511), re-opened
2026-08-31 after being closed-as-completed with nothing implementing it).

**2c and 2d are where key material starts moving again** (the medium carrying wrapped DEKs, the `CAIRNL1`
export carrying the unwrap secret and the actor registry). The newtypes therefore land *before* that code
is written, not retrofitted onto it. They are **not** part of 2a: this slice's crate contains zero
`[u8; 32]`, and the migration touches 83 sites across four crates including `cairn-sync/src/main.rs`, which
would cost 2a the only proof it has — that the extraction changed nothing and every call site compiled
untouched (maintainer decision, 2026-08-31).

### Two constraints that forced this shape

- **The 64 MiB frame cap is now load-bearing.** `EventsAfterSeq` is deliberately *unpaginated*
  ([#101](https://github.com/cairn-ehr/cairn-ehr/issues/101)) — one hex-encoded JSON frame,
  `MAX_FRAME_BYTES = 64 MiB`, about 20k events. A real clinic's `event_log` is far larger. Routing capture
  through the sync path therefore **requires** a batch limit; that is 2b's, not 2a's, but it is why 2a's
  format must be append-shaped rather than whole-set.
- **`event_set_commitment` hashes the whole sorted set**, so the signed self-marker cannot survive an
  append without a re-sign of everything. Hence the segment chain in §5.

---

## 3. What is still broken when 2a merges — read this before quoting it

**2a fixes no defect.** After it merges:

- `backup.rs::read_event_set` still reads `node_event` only. The medium still carries no clinical event.
- `dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event` still **pins the
  defect** and must stay green as a pin. It is inverted in 2d, not here.
- `backup-status.json`, `status` and `verify-backup` still report health and integrity for a medium that
  does not hold the record.

No comment, doc header, test name or commit message introduced by this slice may suggest otherwise. The
recorded lesson this slice is most likely to repeat is exactly the one that let #500 hide: *a deferral is
only honest while its stated precondition holds, and nothing in the repo watches for one expiring.*
Every deferral written here therefore names **the slice that retires it**, not merely "later".

---

## 4. The crate

`crates/cairn-medium` — the backup-medium container format and its markers. Pure: no DB, no I/O, no async.

### Why a crate and not a module

2c and 2d put the clinical plane in `cairn-sync`'s hands (it owns the clinical plane, the wire protocol
and the transport seam), while `medium.rs` lives in `cairn-node` and `cairn-sync` has no dependency on it
— a production dependency on an application crate carrying clap, rustls, rcgen and tokio-postgres is the
wrong direction. This is the same shape #503 resolved by extracting `cairn-keystore`, and it is resolved
the same way.

### Extraction discipline (the #503 pattern)

Today's `crates/cairn-node/src/medium.rs` (706 lines) moves **verbatim** — behaviour-preserving, split only
by responsibility — and `cairn-node::medium` re-exports the surface, so **every existing call site compiles
untouched**. That untouched-call-site property is the extraction's proof, exactly as it was for the 221
`cairn-keystore` call sites.

```
crates/cairn-medium/src/
  lib.rs        module docs; re-exports; the format's invariants in one place
  error.rs      BackupError
  chunk.rs      put_chunk / take_chunk — the [u32 BE len][bytes] primitive
  container.rs  magic dispatch; CAIRNB1/B2 parse; CAIRNB3 section framing
  marker.rs     CAIRNB2 ONLY — SelfMarker, build/verify_self_attestation,
                event_set_commitment. Frozen: it serves existing media and gains
                nothing. Do not extend it; CAIRNB3's equivalent is segment.rs (§5.1).
  segment.rs    CAIRNB3 — records, segments, the attestation chain, self-id     ← new
  verify.rs     VerifyReport, verify_event(s), verify_medium_bytes, the chain pass
```

The split is house rule 4, not taste: the current file is 706 lines and this slice adds to it.

### The one crate-internal dependency

`medium.rs` reaches into `cairn-node` in exactly one place: `crate::identity::NIL_PATIENT`, the zero-UUID
literal used as the `patient_id` of node-plane event bodies. It is a **wire-level constant**, so it moves
to `cairn-event` and `cairn_node::identity` re-exports it — no other call site changes.

### Dependencies

`cairn-event`, `hex`, `thiserror`, `uuid`, `serde_json` — all already in the workspace and already
AGPL-3.0-compatible (house rule 1: nothing new is pulled in).

---

## 5. The format — `CAIRNB3`

`CAIRNB1` (marker-less) and `CAIRNB2` (marker + event frames) **keep parsing exactly as today**; nothing
about an existing medium changes, and `restore`'s current self-detection path is untouched.

`CAIRNB3` is **one uniform structure repeated**: a length-prefixed, chained, plane-tagged segment.

```
CAIRNB3\n
[section]*                            ← repeated to EOF, append-only

section  = [u32 BE len][segment]      ← the length prefix is what makes a torn tail detectable

segment =
  [u8  plane]                         ← 1 = node plane · 2 = clinical plane
  [u32 BE index]                      ← position in the chain, from 0
  [chunk prev_commitment]             ← empty for index 0 — the chain link
  [chunk self_node_id_hex]            ← who wrote it; empty before enrolment
  [chunk attestation]                 ← signed `node.segment_attested`; empty = unsigned (§5.3)
  [u32 BE record_count]
  [record]*

record =
  [chunk signed_bytes]
  [u8  flags]                         ← bit0 attestation · bit1 attester_key · bit2 wrapped DEK
  [chunk attestation]?                ← present iff bit0
  [chunk attester_key]?               ← present iff bit1
  [chunk dek_wrapped]?                ← present iff bit2
  [i64 BE source_seq]                 ← the capturing node's local seq: the medium's cursor
```

### 5.1 The self-marker and the segment attestation are the same object

This is the simplification the design arrived at, and it is worth stating plainly because it removes a
whole class of problem.

CAIRNB2 has a **head marker block** whose signed attestation commits to `event_set_commitment(events)` —
the whole sorted set. That is unappendable by construction: adding one event changes the commitment, so
every append would need the head re-signed, and rewriting the head shifts every byte after it. A two-plane
medium with a head marker is therefore a whole-file rewrite on every backup, which is precisely the cost
the sync-peer decision exists to avoid.

So in CAIRNB3 there is no head block. **Each segment carries its own signed attestation, which names self
and commits to the chain**, and its payload is:

| field | meaning |
|---|---|
| `self_node_id_hex` | who wrote this segment — the marker's job |
| `segment_commitment` | commitment over this segment's records (sorted content addresses, as `event_set_commitment` does) |
| `prev_commitment` | the preceding segment's `segment_commitment` — the chain link |
| `plane`, `index`, `count` | what this segment is and where it sits |

**Self-identification becomes "the last verified segment attestation's `self_node_id_hex`"**, under the
same two binds today's marker has: the named id must be a genesis present in some node-plane segment on
this medium, and that genesis's signer must equal the attestation's signer. The security properties are
preserved exactly — an attacker holds no private key, so a marker can be **withheld** (fail closed to a
manual choice) but never **forged** — and the documented converged-peer splice residual is *narrowed*,
because a spliced segment must also match `prev_commitment` (§7).

2a ships this as a pure function; **2d** wires `restore` to it. Until then no CAIRNB3 medium exists.

### 5.2 One record shape, both planes

A node-plane record is a clinical record with all `flags` bits clear: `node_event` rows carry no
attestation, no attester key and no DEK, and their `source_seq` is `node_event.seq`. One encoder, one
decoder, one set of tests — a second shape would be a second place for a floor check to go stale (the
#173 twin-dispatch lesson).

The clinical fields are **exactly what `EventsResponse` carries on the wire**
(`cairn-sync/src/main.rs`): `events`, `attestations`, `attester_keys`, `wrapped_deks`, `seqs`. That
correspondence is the whole point of the "real sync peer" decision — a medium carrying less would be a
lookalike, and a restore through `apply_remote_event` would silently lose the attestation a suppressing
event needs to be admitted at all.

### 5.3 Unsigned segments

An attestation is empty when the signing key was not available at capture, mirroring the existing
unsigned-marker rule: **an unavailable key never blocks a backup**, it travels flagged. `self_node_id_hex`
is still written, so the operator-typo footgun stays closed exactly as `SelfMarker::Unsigned` closes it
today; what is missing is tamper-evidence, and verification says so. An unsigned segment is never silently
equated with a signed one.

### 5.4 Why explicit chunks and not CBOR

[#521](https://github.com/cairn-ehr/cairn-ehr/issues/521) is live: a `Vec<u8>` without `serde_bytes`
CBOR-encodes as an array of integers, one structural element per byte. Explicit length-prefixed chunks
sidestep that class entirely, match the existing `put_chunk`/`take_chunk` style a reviewer already knows,
and keep the format readable in a hex dump at 3am — which is the hour this format is read.

### 5.5 Unknown planes and unknown fields are NAMED, never skipped

`parse_container` returns `unknown_planes: Vec<UnknownPlane { plane, index, record_count }>` for any
segment whose plane tag it does not recognise. Any consumer that needs completeness — restore,
`verify-backup` — **refuses**, naming the plane and the remedy (*this medium was written by a newer Cairn;
upgrade this node before restoring*).

Skipping an unrecognised segment silently is #500's own failure shape one layer down: a medium that parses
cleanly, reports healthy, and is missing the record. Forward-compatibility here means *degrade honestly*
(principle 4), never *proceed quietly* — and what is returned is a **list of what was not understood**, not
a count, per the standing **NAME, NEVER COUNT** rule.

---

## 6. Append semantics — the medium becomes an append-only log

Because every segment is self-contained, length-prefixed and chained to its predecessor, **capture appends
and never rewrites**: one new section, one new signature, cost O(new records). That is principle 1 applied
to the medium itself, and it is what makes a nightly backup of a growing log affordable.

It gives up the current whole-file atomic rewrite, whose fail-safe property `backup_to` documents carefully
(*"a crash never destroys the previous good medium"*). What replaces it is stronger, not weaker:

- **A torn append is self-limiting.** The section length prefix makes a partial write detectable without
  parsing it; `parse_container` reports `truncated_tail: true` and yields every complete section before it.
  A partial segment carries no valid attestation, so the medium is valid **up to the last verified
  segment**, and the next capture resumes from that point. At most one increment is lost, and it is lost
  *loudly*.
- **Durability is explicit.** Each append is `write` + `sync_all()` **before** the health sidecar advances,
  preserving today's rule that health can only ever under-claim. Not left to the OS; stated as a
  requirement of the writer. **2c** owns the writer — 2a owns the format that makes the guarantee
  expressible.
- **Both planes append.** A new peering appends a node-plane segment; a night's clinical capture appends a
  clinical segment. Set-union makes multiple segments of the same plane simply union, so nothing needs
  rewriting when the node plane grows.

### What "the watermark" means

Per plane: the highest `source_seq` in the last **verified** segment of that plane. Deriving it from
verification rather than from the file's tail is what makes a torn append cost exactly one increment — an
unverifiable trailing segment does not advance the cursor, so its records are re-captured rather than lost.

---

## 7. Verification

`verify_event` keeps its current per-event meaning. `verify_medium_bytes` gains a **chain pass**:

1. every segment's attestation verifies, or is honestly absent (§5.3);
2. each segment's `segment_commitment` matches its own records;
3. each segment's `prev_commitment` equals the preceding segment's `segment_commitment`;
4. each signed segment's `self_node_id_hex` is bound as in §5.1.

A break is reported **by segment plane and index** — *"clinical segment 7 breaks the chain"* sends an
operator somewhere; *"chain invalid"* does not. **The index is medium-wide, not per-plane**: the built
chain is **one global chain in file order** over both planes together, not two independent per-plane
chains. One chain is what lets it detect a reorder or a splice **across** planes — a segment lifted from
the node plane and reinserted into the clinical plane's position, or vice versa — which two chains, each
blind to the other's positions, could not. `VerifyReport` grows these fields **additively**, so every
existing caller keeps compiling and keeps meaning what it meant.

The chain also narrows the documented converged-peer splice: a genuine segment lifted from another medium
commits to a different predecessor and fails at (3). It does not eliminate the residual for a medium
spliced at index 0, which remains restore-time provenance's job (`Provenance::SignedFederated`) — **2d**.

---

## 8. Out of scope — deliberately, with the slice that retires each

- **Writing real clinical records.** No DB read exists in this crate by construction. → **2c**.
- **The transport seam and the paged pull.** `MediumTransport`, the `EventsAfterSeq` batch limit. → **2b**.
- **The actor registry in `CAIRNL1`.** → **2d**.
- **Health and scope honesty** in `backup-status.json` / `status` / `verify-backup`. → **2c**.
- **The superseding ADR.** → **2e**.
- **Encryption of the medium.** ADR-0026 assumes an encrypted volume; the container is not itself sealed
  today and 2a does not change that. Not a regression, and not this slice's question.
- **[#101](https://github.com/cairn-ehr/cairn-ehr/issues/101) pagination on the network path.** 2b adds the
  batch limit the medium needs; whether the *network* full-sweep adopts it stays #101's call.

---

## 9. Testing (TDD — failing test first, in this order)

All pure, no DB, no I/O. Fixtures use real `cairn-event`-signed events, and **every key is derived at
runtime** (`generate_key()` / `from_fn`), never a literal — house rule 6, because a literal in a crypto
context trips CodeQL's `rust/hard-coded-cryptographic-value` and blocks the scan.

**The extraction's floor, written before the move:**

1. **CAIRNB1 and CAIRNB2 round-trip unchanged**, and `verify_self_attestation` still accepts a genuine
   marker and still rejects a tampered one, a foreign-set one, and one whose signer is not the named
   genesis's. This is the regression floor for a verbatim move — it must pass before and after.

**The new format:**

2. **A CAIRNB3 segment round-trips byte-exact**, both planes, through `serialize` → `parse`.
3. **One record shape covers both planes** — a node-plane record (all `flags` clear) and a clinical record
   with all three optional fields both survive; each of the eight `flags` combinations round-trips.
4. **An absent optional field decodes as `None`, never as an empty `Vec`** — an empty attestation and a
   missing one are different facts, and conflating them is how a fail-closed gate becomes fail-open.
5. **The chain holds and locates its break** — a valid multi-segment medium verifies; a medium whose
   segment 3 of 6 has a mangled `prev_commitment` is reported **by plane and index**, not as a bare count.
6. **A segment spliced from another medium fails `prev_commitment`** even though its own signature and
   commitment are genuine — the splice narrowing claimed in §7.
7. **A torn tail yields the verified prefix plus the flag**, and — the assertion that matters — the derived
   watermark is the last *verified* segment's, so the torn increment is re-captured rather than lost.
8. **An unsigned segment verifies as unsigned and still carries `self_node_id_hex`** — never silently
   equated with a signed one, and never losing the operator-typo protection today's unsigned marker gives.
9. **Self-identification takes the last verified segment attestation**, binds the named id to a genesis
   present in a node-plane segment, and binds that genesis's signer to the attestation's signer. A
   withheld or corrupt attestation **fails closed** (no self id) rather than resolving to a wrong one.
10. **An unrecognised plane tag is named, not skipped** — `unknown_planes` carries plane, index and record
    count, and a completeness-requiring consumer refuses.
11. **A tampered record's signature fails** and `first_bad` names its position.

**Mutation discipline.** For every assertion above, confirm the property actually fails when the production
line it guards is inverted. Two recorded lessons apply directly: *a mutation that does not change the
property tests nothing*, and PR #410's green suite survived **7 of 11** production mutations. Tests 4, 7
and 9 are the ones most likely to pass vacuously — check those first.

---

## 10. Paper-parity (§1.2)

**Not clinical-surface** — a pure container format with no operator-visible act and no clinical workflow at
any layer. The DR ceremony changes in **2c** (capture) and **2d** (restore), and those plans carry the
benchmark; [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512) already records restore's `M = 3`
against paper's `N = 2` and must not be made worse by them.

---

## 11. Mechanical obligations

- **Three Cargo trees, three lockfiles.** A new root-workspace member makes `cairn-gui/Cargo.lock` and
  `extensions/cairn_pgx/Cargo.lock` stale. No root-workspace gate sees this — CI's `--locked` clippy on the
  GUI tree is the only thing that does, and `--locked` refuses to regenerate. Refresh all three.
- **`PRODUCTION_TREES`** in `unwrap_secret_is_not_derived.rs` sweeps `crates/` already, so the new crate is
  covered with no allow-list change — and `cairn-medium` calls `derive_unwrap_secret` nowhere, so nothing is
  added to `ALLOWED`.
- **Pinned counts.** This slice registers no event type and adds no `cairn_decode_hex_or_raise` call site,
  so the twin/projection registries and `hex_decode_helper.rs`'s per-file list are untouched. Re-check
  before merge rather than assuming.
- **Gate cost.** A new crate touches `Cargo.lock`, which relinks the whole test-binary set; on macOS each
  binary draws a one-time Gatekeeper assessment. **Budget hours, not minutes**, and start the full local gate
  in the background. `cargo test -p cairn-medium -p cairn-node` is the confined run during development.
- **`--all-targets`, `--no-fail-fast`, and never pipe cargo to `tail`** — all three hiding modes are recorded
  in this repo's history.

---

## 12. Deferred questions this slice deliberately does not answer

- **Does the medium or the export own custody?** The record shape reserves `dek_wrapped` per event, and
  `CAIRNL1` already carries `episode_deks` with the `erasure_shred_log` filter (ADR-0066 decision 7). Both
  paths must apply the same shred exclusion or a shredded body comes back. **2c decides which is
  authoritative**; 2a only makes both expressible. This is the single most dangerous loose end in the
  programme and is named here so it cannot be lost.
- **Whether an unsigned segment should ever be restorable without operator confirmation.** Today's unsigned
  *marker* is surfaced as `Provenance::Unsigned` for confirmation; segments should probably inherit that.
  **2d decides.**
- **Streaming parse.** `parse_any` reads a whole image into memory. The section framing is
  exactly what makes a streaming reader possible as a later, purely additive change, but a
  medium larger than RAM cannot be parsed today. **2b decides**, since `MediumTransport` is
  the first consumer that can meet one.
