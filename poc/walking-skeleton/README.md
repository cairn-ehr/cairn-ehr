# Cairn walking skeleton — Spike 0001

The smallest thing that is *genuinely* Cairn's architecture rather than a mock of
it: a signed, append-only event envelope; set-union sync that verifies on apply;
a trigger-maintained projection; and a content-addressed lazy blob tier. It is the
shared prerequisite for both bets in
[Spike 0001](../../docs/spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md) and,
per that spike's §7, the **seed of the real implementation** — built to be the
architecture, not thrown away.

Sibling to [`poc/replication-failover`](../replication-failover/), which proved
set-union convergence with a *bare* event log. This skeleton adds the parts that
PoC deliberately omitted: the **signed COSE_Sign1 envelope**, an **in-database
content-address invariant**, **verify-on-apply**, a **trigger-maintained
projection** (so Bet B has a real maintenance path to measure), and the
**BLAKE3 content-addressed blob tier** (Bet A4).

## What it is (and the §9 blast-radius mapping)

| Piece | Where | Substrate (per [§9](../../docs/spec/language-substrate.md)) |
|---|---|---|
| Event envelope, content-address invariant, append-only enforcement | `db/001_envelope.sql` | safety → in-database |
| Trigger-maintained projection (`patient_chart`) | `db/002_projection.sql` | safety → in-database (PL/pgSQL; pgrx hatch if Bet B needs it) |
| Content-addressed blob store + self-verifying CHECK | `db/003_blobs.sql` | safety → in-database |
| Canonical bytes · COSE_Sign1/Ed25519 sign+verify · multihash · BLAKE3 | `crates/cairn-event` | safety → Rust |
| Thin set-union ship/apply daemon + lazy blob tier | `crates/cairn-sync` | safety → Rust (no merge logic — [ADR-0001](../../docs/spec/decisions/0001-fat-postgres-thin-daemon.md)) |

The three structural moves from Spike 0001 §4 are concrete here: events are signed
**verbatim bytes** that are never re-serialized; digests are **algorithm-tagged
multihashes**; and the verify gate is the **one safety-critical seam** that moves
in-DB via pgrx ([ADR-0002](../../docs/spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md))
in production so no unverified row can enter the log.

## Build & test

```sh
cargo test --workspace      # unit tests incl. sign→wire→verify round-trip + tamper detection (Bet A2)
cargo build --workspace     # produces target/debug/cairn-sync
```

Requirements: a recent Rust toolchain and PostgreSQL (the project floor is **≥ 18**;
the skeleton's SQL also runs on 16 for local testing — it uses no 18-only syntax,
since UUIDv7s are minted in Rust, not via `uuidv7()`). `pgcrypto` is created by
`init`. Sync runs over plain TCP with **NoTls on purpose**: the deployment
transport is **WireGuard**, which is the link's encryption (Spike 0001 assumption).

## Run it — two nodes on one machine

```sh
BIN=target/debug/cairn-sync
A="host=127.0.0.1 user=postgres dbname=skeleton_a"
B="host=127.0.0.1 user=postgres dbname=skeleton_b"

$BIN init --conn "$A"; $BIN init --conn "$B"

# Partition: each node writes independently (no link yet).
PID=$(psql "$A" -tAc "select gen_random_uuid();")
$BIN write --conn "$A" --node cape-york --key a.key --type patient.created --patient "$PID" \
     --schema patient/1 --json '{"name":"Alma Tjapaltjarri","dob":"~1956","sex":"F"}'
$BIN write --conn "$B" --node dorrigo  --key b.key --type note.added --patient "$PID" \
     --schema note/1 --json '{"text":"Phone consult from Dorrigo."}'

# Reconnect: both serve, both pull (set-union, verify-on-apply).
$BIN serve --conn "$A" --listen 127.0.0.1:7710 &
$BIN serve --conn "$B" --listen 127.0.0.1:7711 &
$BIN pull  --conn "$A" --peer 127.0.0.1:7711 --peer-name dorrigo
$BIN pull  --conn "$B" --peer 127.0.0.1:7710 --peer-name cape-york
# event_log now identical on both nodes; patient_chart reflects both notes.
```

Blob byte-tier (Bet A4): `put-blob` stores bytes on one node; the other learns the
reference and fetches lazily. The byte tier is **resumable** (verified slices
persist in `blob_chunk`; a restart fetches only missing indexes), **windowed**
(concurrent slice workers, `--window N`, ≤16 to protect the availability floor),
**swarm-capable** (repeat `--blob-peer HOST:PORT` for multiple sources), and
**per-slice verified** (every slice is checked against the BLAKE3 `blob_address`
via bao verified streaming; a lying or faulty source is rejected per-slice and
healed by another source). Slices travel as **raw binary frames**
(`[found:u8][total_len:u64 BE][slice…]`), not hex — the byte tier is
throughput-bound, and hex would double every transferred byte; the clinical plane
stays JSON because it is small and latency-bound. The `--budget-ms` sleep between
requests is preserved to keep byte transfer from starving clinical sync.

A single `blobd` call makes **one pass** over the missing slices and returns; a
transient link drop simply leaves an index missing for the next pass (resumable).
Re-run `blobd` until `blobs_completed` covers your references, or use `run`, whose
byte-tier thread loops automatically.

```sh
$BIN gen-blob --conn "$A" [--size-mb N] [--media MEDIA_TYPE]          # mint a large local blob to fetch
$BIN put-blob --conn "$A" --file scan.dcm --media application/dicom   # prints the blob address
psql "$B" -c "select blob_note_reference(decode('<addr>','hex'),'application/dicom', <len>);"
$BIN blobd --conn "$B" \
    --blob-peer 127.0.0.1:7710 [--blob-peer HOST:PORT ...] \          # repeatable; swarm sources
    [--window N] [--budget-ms 20] [--metrics]                         # N ≤ 16; default budget-ms 20
```

`serve` accepts an optional `--corrupt` flag (**TEST-ONLY** fault injection — flips
a byte of each served slice so the receiver's per-slice verify rejects it; used by
`harness/bench_blob.py` to prove swarm self-heal; never set on a real node).

For the real two-machine run (Cape York ↔ Dorrigo over WireGuard), point `--peer`
at the peer's WireGuard address and follow the pattern in
[`poc/replication-failover/TWO-MACHINE-RUNBOOK.md`](../replication-failover/TWO-MACHINE-RUNBOOK.md).

## What this skeleton proves (and what it deliberately stubs)

**Demonstrated end-to-end on real PostgreSQL:** schema load · the content-address
CHECK rejecting a tampered row · sign → wire → verify-on-apply · bidirectional
set-union **convergence to an identical event set + HLC order** · idempotent
re-pull · watermark-0 re-pull still converging (the watermark is a hint, not an
authority — [ADR-0004](../../docs/spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)) ·
correct projection under **out-of-order** apply · BLAKE3 blob fetch with
**resumable/windowed/swarm** slice fetch + per-slice verified streaming ·
a self-verifying `blob_store` CHECK on both ends · **swarm self-heal** (lying-peer
fault injection via `serve --corrupt` → per-slice rejection → heal from good peer).

**Stubbed on purpose** (Spike 0001 §2 — absence doesn't change the bet):
- **Verification trusts the key embedded in the event.** Production resolves
  `signer_key_id` against the enrolled actor registry ([ADR-0011](../../docs/spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md)) —
  origin is proven by signature, but *which* keys are trusted is a registry decision.
- **Change capture is watermark-pull, not logical decoding.** Convergence semantics
  (set-union + idempotent apply) are identical; [§6.1](../../docs/spec/decisions/0001-fat-postgres-thin-daemon.md)
  logical decoding is the production change-capture optimization.
- **The verify gate runs in the Rust applier, not in-DB.** The pgrx move (ADR-0002)
  is what makes it unbypassable; the content-address CHECK is the part already in SQL.
- **Blob bytes are inline BYTEA** (not a dedicated object store). The fetch protocol
  is now windowed, resumable, swarm-capable, and per-slice verified (Spike 0001 §8.2
  resolved), but the storage substrate remains BYTEA; true N-way swarm is only
  locally exercisable on the two-node rig. A consequence of BYTEA storage: the
  **server reads the whole blob from Postgres for every slice request** (bao seeks
  within an in-memory cursor), so serving an N-slice blob does N full-blob DB reads.
  This is **local** I/O on the serving node (not over the WAN link being measured),
  but the production object-store tier replaces it with a ranged read.
- **The byte tier has no per-blob authorization.** `serve` ships any slice of any
  present blob to any peer that can reach the port, exactly as the clinical plane
  ships every event — the WireGuard link is the trust boundary here. Visibility
  scope, the safety projection, and break-glass ([ADR-0006](../../docs/spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md))
  are **not** exercised by this skeleton.
- **Sealing/crypto-shred** (`sealed`/`dek_wrapped`) and rich **contributor sets**
  are reserved in the envelope but not exercised.

## Bet A measurement harness

`harness/bet_a.py` (stdlib only — no pip) drives the binary to emit the Spike 0001
§5 pass/fail table directly against thresholds. The daemon grew three commands for
it: `gen` (bulk load generator), `fingerprint` (convergence/honest-state JSON), and
`pull --metrics` (per-pull JSON: verify-failures, bytes/event, latency).

```sh
# Self-contained: the whole §5 table on two local databases.
python3 harness/bet_a.py selftest \
    --conn-a "host=127.0.0.1 user=postgres dbname=skeleton_a" \
    --conn-b "host=127.0.0.1 user=postgres dbname=skeleton_b"
```

It measures: **A1** convergence (event + projection hash identical across nodes),
**A2** zero verify-failures on apply, **A3** the HLC merge invariant (with the
HLC↔record gap reported, never auto-resolved), **A4** the availability floor
(clinical pull p95 during a concurrent blob fetch vs. baseline), **A5** bytes/event
on the clinical plane, **A6** honest assembly-state. Exit code 0 = all PASS.

> [!NOTE]
> Single-box `selftest` validates the **mechanics**; **A4 is only meaningful on a
> real shared link** (there is no bandwidth to contend for on one box).

### Unattended field run (the real Cape York ↔ Dorrigo test)

`cairn-sync run` serves, pulls, and fetches blobs on a timer, **survives link drops**
(bounded connect + retry/backoff; a sustained outage is logged as a partition, never
fatal), and appends **one JSON line per cycle** to a log — so you start it and walk
away for hours of real Starlink variability, then analyse the log later.

On each node (point `--peer` at the *peer's* WireGuard address):

```sh
# IMPORTANT: --listen on the WireGuard address (or 0.0.0.0), NOT 127.0.0.1,
# or the peer can't reach you.
cairn-sync run --conn "$CONN" \
    --listen 10.0.0.1:7710 --peer 10.0.0.2:7710 --peer-name dorrigo \
    --interval-ms 2000 --log capeyork.jsonl
    # runs until killed (--duration-s 0); add optional flags as needed:
    #   --blob-peer 10.0.0.2:7710 ...   (repeatable; swarm sources)
    #   --window N                       (windowed blob fetch, N ≤ 16)

# meanwhile, generate clinical load on each node (a separate terminal):
cairn-sync gen --conn "$CONN" --node capeyork --key node.key --count 100000 --rate 2
```

When you're back, turn each node's log into the §5 numbers, then compare the two
final fingerprints for convergence (A1):

```sh
python3 harness/bet_a.py analyze --log capeyork.jsonl     # A2/A4-latency/A5/A6 + partition behaviour
python3 harness/bet_a.py analyze --log dorrigo.jsonl
python3 harness/bet_a.py report  --local capeyork.jsonl.fingerprint.json \
                                 --peer  dorrigo.jsonl.fingerprint.json   # A1 + A3
```

`analyze` reports duration, **partition cycles** (how often the link was down), pull
latency p50/p95/max, A2 verify-failures, A5 bytes/event, A3 HLC merge + gap, and A6
blob present/referenced-only — and writes a `.fingerprint.json` for the A1 compare.

## Bet B benchmark harness (the Pi compute-cost bet)

`harness/bench_b.py` (no Python deps; shells out to `psql`, present on any PG node)
drives the binary to emit the §6 table. The daemon grew three commands for it:
`bench-insert` (B1 — maintained-write latency at the current log size), `chart`
(B2 — full chart assembly from the projection + the plaintext legibility twins), and
`bench` (B3/B4 — pure-CPU crypto: Ed25519 sign/verify, SHA-256 vs BLAKE3, DEK-wrap/body-seal).

```sh
cargo build --release          # REQUIRED — debug crypto/projection numbers are meaningless
# Run ON THE PI, against its local PostgreSQL.
# selftest DROPs+recreates the Cairn tables, so it requires --force (guards a mistyped --conn):
python3 harness/bench_b.py --bin target/release/cairn-sync selftest --force \
    --conn "host=127.0.0.1 user=cairn dbname=pi" --sizes 5000 50000 200000
# just the pure-CPU crypto numbers (B3/B4), no DB, non-destructive:
python3 harness/bench_b.py --bin target/release/cairn-sync bench
```

It measures: **B1** single-op projection maintenance and *that it stays flat as the
log grows* (the ADR-0001 load-bearing bet, gated), **B2** chart read beats "grab the
paper chart" (sub-second, gated), **B3** keystore cost (DEK-wrap/body-seal → per-event
vs per-episode crypto-shred granularity — reported as INFO, not gated), **B4** Ed25519
verify/s + SHA-256-vs-BLAKE3 (the ARM input to ADR-0015's *provisional* blob-digest
default, gated). On a miss it prints the ADR-0002 mitigation ladder (PL/pgSQL → pgrx →
external Rust). Run it **on the Pi**; single-machine numbers reflect whatever ran them.

## Byte-tier throughput harness (Spike 0001 §8.2)

`harness/bench_blob.py` (stdlib only) exercises the windowed/resumable/swarm/lying-peer
byte tier: throughput vs window size, resume-across-drop, swarm fetch from multiple
sources, and swarm self-heal (lying peer → per-slice reject → heal from good source).
Also validates the availability floor (byte transfer must not starve clinical sync).

```sh
# Three connections: node A (source A), node B (source B, second swarm source), node C (fetcher/dst).
# selftest DROPs+recreates blob data — requires --force to guard a mistyped --conn.
python3 harness/bench_blob.py selftest \
    --conn   "host=127.0.0.1 user=postgres dbname=skeleton_a" \
    --conn-b "host=127.0.0.1 user=postgres dbname=skeleton_b" \
    --conn-c "host=127.0.0.1 user=postgres dbname=skeleton_c" \
    --force
```

## Next (the spike's bets)

- **Bet A — DONE** (#9): all six §5 rows PASS over the real Cape York ↔ Dorrigo
  link; §4 primitives ratified as [ADR-0015](../../docs/spec/decisions/0015-event-serialization-signatures-and-content-addressing.md).
- **Bet B — harness ready:** run `bench_b.py selftest` on a Pi-5-class node. The
  ARM SHA-256-vs-BLAKE3 number is the one input that could revisit ADR-0015's
  provisional blob-digest default.
- **Byte-tier throughput — DONE** (spike §8.2): windowed/resumable/swarm/verified
  fetch shipped; `harness/bench_blob.py` is the selftest harness.
