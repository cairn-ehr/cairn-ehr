# Pi runbook — Bet B, the compute-cost go/no-go on weak hardware

This is the field guide for running **Bet B** of
[Spike 0001](../../docs/spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md) on a
real Raspberry-Pi-class node — the half the Cape York ↔ Dorrigo WAN run (Bet A)
*couldn't* stress, because both those machines are fast. Bet B is the documented
go/no-go on the [ADR-0001](../../docs/spec/decisions/0001-fat-postgres-thin-daemon.md)
bet: **that trigger-maintained in-DB projections + signed-event verification stay
cheap enough on a rural-clinic Pi to keep chart reads local and faster than grabbing
the paper chart** ([§1.2](../../docs/spec/vision.md) paper-parity floor).

The harness and the daemon commands it drives are already built and green (on x86);
what only *you* can do is run them on the actual board. Follow this start to finish
when you pull the Pi out of the drawer.

> [!NOTE]
> **A "slow" result is not a design failure.** A miss tells you *which rung* of the
> [ADR-0002](../../docs/spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)
> mitigation ladder (PL/pgSQL → pgrx in-DB Rust → external Rust) the hot projection
> needs — not whether the architecture works. The one result that would feed back
> into a *spec* decision is the B4 ARM **SHA-256-vs-BLAKE3** number, which touches
> [ADR-0015](../../docs/spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)'s
> *provisional* blob-digest default.

---

## 0. What this run answers

Four questions, the [§6](../../docs/spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#6-bet-b--projection--keystore-cost-on-the-pi-next-week)
table, emitted as a PASS/FAIL/INFO grid by `harness/bench_b.py`:

| # | Question | Gate |
|---|---|---|
| **B1** | Is projection maintenance cheap, and does it **stay flat as the log grows**? | p95 maintained-write within budget; growth ≤ ×N across a big log-size jump |
| **B2** | Does a realistic **chart read beat paper**? | sub-second p95 |
| **B3** | What does the **keystore** (crypto-shred DEK-wrap/body-seal) cost? | INFO — informs per-event vs per-episode key granularity ([ADR-0005](../../docs/spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md)) |
| **B4** | Does **crypto keep up on ARM**? | Ed25519 verify/s over floor; BLAKE3-vs-SHA-256 (the ADR-0015 input) |

**And the floor question.** You suspect the Pi 5 / 16 GB is the realistic minimum.
The gates answer go/no-go on *this* board; the **headroom** the harness prints next to
each gated row (how many × under budget / over floor you landed) is what predicts
whether a *smaller* board would still pass — so the Pi 5 run already informs the floor
question before the second board is plugged in (§8).

---

## 0.5 What you need

* A **Raspberry Pi 5 (16 GB)** — the target you have in the drawer.
* A **1 TB SSD**, and the means to attach it: the official **M.2 NVMe HAT** (or any
  PCIe-M.2 adapter) is best; a good **USB-3 SATA/NVMe enclosure** is fine. The point
  of the SSD for *this* run is two-fold: (a) **PGDATA must live on it, never on the SD
  card** (§1), and (b) it makes the big-log B1 tiers (§6) feasible. (Capacity is not
  the constraint — see the sizing appendix; the bet is CPU, not bytes.)
* **Active cooling** (the official active cooler or a heatsink+fan). A Pi 5 under a
  sustained crypto/insert load *will* thermally throttle without it, and a throttled
  number is not the hardware's real ceiling — the harness flags this, but cooling
  avoids the problem.
* This repo checked out on the Pi, a Rust toolchain, and **PostgreSQL 18** (§2–§5).
* A quality **5 V / 5 A USB-C PSU** (the official 27 W). Brown-outs also show up as
  throttle flags and corrupt the run.

---

## 1. Put PGDATA on the SSD — not the SD card (the one Pi mistake that invalidates B1/B2)

A microSD card is slow and wears out; B1 (write/projection-maintenance latency) and
B2 (chart read) measured against an SD card measure *the SD card*, not Cairn. So the
PostgreSQL **data directory must sit on the SSD.** Two equally fine ways:

* **Boot the whole Pi from the SSD** (Pi 5 supports NVMe boot via the HAT). Cleanest;
  then everything, including PGDATA, is already on the SSD.
* **Boot from SD, put only PGDATA on the SSD.** Mount the SSD and point Postgres at
  it (§3). Perfectly valid for the benchmark.

Mount and confirm the SSD (adjust the device — `lsblk` shows it; NVMe is `/dev/nvme0n1`,
USB is usually `/dev/sda`):

```bash
lsblk -o NAME,SIZE,TYPE,MOUNTPOINT,MODEL
# Format once if blank (DESTROYS the disk — be sure of the device):
sudo mkfs.ext4 /dev/nvme0n1     # or /dev/sda
sudo mkdir -p /mnt/ssd
echo "/dev/nvme0n1 /mnt/ssd ext4 defaults,noatime 0 2" | sudo tee -a /etc/fstab
sudo mount -a
df -h /mnt/ssd                  # confirm it's the 1 TB device
```

> [!NOTE]
> The harness **detects this for you**: its environment header maps PGDATA to its
> backing block device and shouts if it sees `/dev/mmcblk*` (an SD card). You do not
> have to remember — but you do have to fix it if it warns.

---

## 2. Install PostgreSQL 18 (arm64)

Raspberry Pi OS is Debian-based and the PostgreSQL project ships arm64 packages, so
PG 18 — the project floor — installs cleanly from the PGDG apt repository:

```bash
sudo apt-get update && sudo apt-get install -y curl ca-certificates gnupg lsb-release
sudo install -d /usr/share/postgresql-common/pgdg
sudo curl -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc \
     https://www.postgresql.org/media/keys/ACCC4CF8.asc
echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] \
http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" \
  | sudo tee /etc/apt/sources.list.d/pgdg.list
sudo apt-get update
sudo apt-get install -y postgresql-18
psql --version       # expect 18.x
```

(The skeleton's SQL uses no 18-only syntax — UUIDv7s are minted in Rust — so it *runs*
on 16 too, but record the real Bet B numbers on **18**, the deployment floor. The
harness warns if it sees < 18.)

---

## 3. Create the benchmark database on the SSD

Put the cluster's data directory on the SSD. Simplest is a fresh cluster there:

```bash
sudo mkdir -p /mnt/ssd/pgdata && sudo chown postgres:postgres /mnt/ssd/pgdata
sudo -u postgres /usr/lib/postgresql/18/bin/initdb -D /mnt/ssd/pgdata
# Start it on a port of your choosing (5444 keeps it clear of any distro default):
sudo -u postgres /usr/lib/postgresql/18/bin/pg_ctl -D /mnt/ssd/pgdata \
     -o "-p 5444" -l /mnt/ssd/pg.log start
# A role + db for the run:
sudo -u postgres psql -p 5444 -c "create role cairn login superuser;"
sudo -u postgres psql -p 5444 -c "create database pi owner cairn;"
```

Your harness connection string is then:

```bash
CONN="host=127.0.0.1 port=5444 user=cairn dbname=pi"
```

---

## 4. Make the measurement honest (governor, cooling, tuning)

Three things turn a noisy Pi into a fair, repeatable benchmark. The harness **records
all three in its environment header**, so even if you skip a step the number is at
least self-describing — but for the real run, do them.

**CPU governor → `performance`** (otherwise the Pi clocks down between samples and you
measure power-saving, not the ceiling):

```bash
sudo apt-get install -y linux-cpupower
sudo cpupower frequency-set -g performance
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor   # -> performance
```

**Confirm it isn't throttling** (needs active cooling + a real PSU):

```bash
vcgencmd get_throttled        # -> throttled=0x0 means clean
vcgencmd measure_temp
```

**PostgreSQL tuning for a 16 GB Pi.** Modest, deployment-honest settings (do **not**
disable `fsync`/`synchronous_commit` — a rural clinic node must survive power loss, so
measure the safe configuration). **On an 8 GB board** (what both the 2026-06-25 and
2026-07-07 runs actually used) halve the memory knobs — `shared_buffers = '2GB'`,
`effective_cache_size = '5GB'` — and keep the rest:

```bash
sudo -u postgres psql -p 5444 -d pi <<'SQL'
ALTER SYSTEM SET shared_buffers = '4GB';            -- ~25% of 16 GB
ALTER SYSTEM SET effective_cache_size = '10GB';
ALTER SYSTEM SET work_mem = '64MB';
ALTER SYSTEM SET max_wal_size = '4GB';
ALTER SYSTEM SET wal_compression = 'on';            -- WAL bytes are scarce on an off-grid link
-- fsync / synchronous_commit left at the safe default (on) on purpose.
SQL
sudo -u postgres /usr/lib/postgresql/18/bin/pg_ctl -D /mnt/ssd/pgdata -o "-p 5444" restart
```

---

## 5. Build the daemon (release, on the Pi)

The 16 GB Pi 5 builds the workspace natively without trouble. **Release is mandatory** —
debug Ed25519/BLAKE3/projection numbers are an order of magnitude off and meaningless.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # if Rust isn't installed
source "$HOME/.cargo/env"
sudo apt-get install -y build-essential pkg-config libssl-dev postgresql-client

cd <repo-root>                   # the cargo workspace lives at the repo root, NOT poc/walking-skeleton
cargo build --release -p cairn-sync   # produces target/release/cairn-sync (arm64)
cargo test --release             # sanity: the 6 crypto/round-trip tests should pass
```

> [!NOTE]
> **Build the extension for PG 18** (this is the deployment floor and the default). `cairn_pgx` now targets
> **pgrx 0.18.1** with a `pg18` feature, so the whole run — including the DB gates B1/B2/B5 — happens on
> **PostgreSQL 18** (the 2026-06-25 PG-16-only caveat is resolved; see [§9.5](../../docs/spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#95-clean-re-run-pg-18-nvme-2026-07-07-pass-both-caveats-resolved)).
> Two build prerequisites the 2026-07-07 re-run surfaced on a fresh board:
> 1. **`cargo-pgrx` must match the pgrx crate** — upgrade it: `cargo install cargo-pgrx --version 0.18.1 --locked --force`.
> 2. **Register the system PG 18 with pgrx** before building, or the `pgrx-pg-sys` build fails with *"Postgres
>    `pg18` is not managed by pgrx"*: `cargo pgrx init --pg18 /usr/lib/postgresql/18/bin/pg_config`.
>
> Then build+install the extension on the board (needs `postgresql-server-dev-18` + `libclang-dev`):
> `LIBCLANG_PATH=/usr/lib/llvm-19/lib cargo pgrx install --no-default-features --features pg18
> --pg-config /usr/lib/postgresql/18/bin/pg_config --release --sudo`. The **crypto-only B3/B4 `bench`** needs
> no DB and runs fine regardless.

---

## 6. Run Bet B

`harness/bench_b.py` has no Python dependencies (it shells out to `psql`). Run it **on
the Pi**, against the local PG 18 from §3. It prints the self-describing environment
header, then the §6 table; `--json-out` records the whole thing for the spike log.

**Quick crypto-only look first** (B3/B4, no DB, a few seconds — confirms the binary,
cooling, and governor before the long load):

```bash
python3 harness/bench_b.py --bin target/release/cairn-sync bench \
    --label "pi5-16gb-nvme" --json-out /mnt/ssd/betb-pi5-crypto.json
```

**The full §6 run** with the prescribed Pi size ladder. The big tiers are what prove
B1 stays *flat* as the log grows (the ADR-0001 bet) and what put the SSD to work;
scale `--patients` with the log so the fattest patient's chart (B2) stays a *realistic*
size (the chart is ~ `count / patients` notes — ~500 notes per patient below is a heavy
multimorbid chart, not a degenerate one):

```bash
python3 harness/bench_b.py --bin target/release/cairn-sync selftest --force \
    --conn "$CONN" \
    --sizes 50000 500000 2000000 \
    --patients 4000 \
    --label "pi5-16gb-nvme" \
    --json-out /mnt/ssd/betb-pi5.json
```

> [!NOTE]
> **Generating the large tiers is a one-time bulk load and can take a while** on the Pi
> (each event is a signed round-trip). It is *not* part of the measured path — B1 times
> a fresh batch of writes *at* each size, and B2 times chart reads — so start it and let
> it run. If you're impatient on the first pass, drop to `--sizes 20000 200000` to get a
> verdict, then do the 2 M tier once for the real flatness number.

`--force` is required because `selftest` drops and recreates the Cairn tables (a
benchmark needs a known-empty log); it guards against a mistyped `--conn`.

### 6.1 B5 — does surrogate-key interning pay on ARM? (ADR-0031)

[ADR-0031](../../docs/spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)
keeps the canonical UUID on the wire but interns it to a node-local `bigint`
surrogate as the physical join key. B5 measures whether the smaller foreign-key
index actually pays on the Pi. It is a **pure-SQL** run (no Rust rebuild, no pgrx) —
the discipline lives entirely in the projection plane — so it is separate from the
B1/B2 daemon run above and does not perturb their numbers.

Run it against a **throwaway** bench database (it `TRUNCATE`s the projections):

```bash
# A fresh bench DB on the SSD cluster from §3:
psql "$CONN" -c "CREATE DATABASE cairn_b5" 2>/dev/null || true
B5="host=127.0.0.1 port=5444 user=cairn dbname=cairn_b5"

# Loads 001+002+008, runs the leakage/interning guard, then the size/read bench.
# Scale the second/third args up on the Pi to a realistic fleet (patients, notes each):
db/bench/run_b5.sh "$B5" 20000 100
```

Read **B5.1**'s `shrink_factor` (the foreign-key index size ratio — the cost ADR-0031
targets) and confirm **B5.4**'s surrogate read stays competitive with **B5.3**'s direct
UUID read once the one-row anchor rehydrate is counted. A "no material shrink / slower
read" result on ARM **narrows** the interning scope (keep UUIDv7-only where it doesn't
earn its indirection); it does not overturn the discipline. Record the numbers in §9
alongside the B1–B4 table.

> [!NOTE]
> The guard (`db/tests/008_surrogate_test.sql`) is the load-bearing half: it mechanically
> asserts the surrogate never reaches the canonical/signed plane (`event_log` stays
> surrogate-free — the load-bearing guarantee, since that plane is typed `uuid` and
> `bigint <> uuid`), that the `local_ref` domain gives its one-directional guard (a
> `uuid` won't coerce to a surrogate) while being honest it is *not* a two-way barrier,
> and that egress rehydrates the canonical UUID. It runs anywhere `psql` does — including in review, off-Pi.

---

## 7. Read the result, and the floor

The harness prints, per gated row, both the measurement and its **headroom**:

```
B1  projection maintenance ...  PASS  p95 X ms @ 2,000,000 events (Nx under budget 50ms); growth x… (flat<=x3.0)
B2  chart read beats paper      PASS  p50 … p95 Y ms over ~500 notes (Mx under budget 1000ms)
B3  keystore cost ...           INFO  DEK-wrap …/s, body-seal … MB/s
B4  crypto on ARM ...           PASS  Ed25519 … verify/s (Kx over floor); BLAKE3 … vs SHA-256 … MB/s (…)
```

How to read it for the **go/no-go** and the **floor**:

* **All PASS → go on this hardware.** The Pi 5 / 16 GB clears the ADR-0001 bet at the
  lowest (PL/pgSQL) rung.
* **The headroom multipliers are the floor signal.** Big headroom on B1/B2 (say tens of
  × under budget) means the projection/chart path is nowhere near the constraint, so a
  weaker board likely still passes those — the floor is then set by whichever row has
  the *smallest* headroom (often **B4**, raw crypto throughput, which scales with clock
  and core, not with tuning). Small headroom anywhere is the row to watch on the smaller
  board.
* **A B1/B2 miss → mitigation ladder, not a redesign.** Re-read the harness's printed
  remedy: PL/pgSQL → pgrx (in-DB Rust for the hot projection) → external Rust. Record
  which rung cleared it.
* **A B4 miss → a hardware-class signal**, not a projection fix: a faster node, or
  (for the blob digest specifically) accept SHA-256 — ADR-0015's provisional line.
* **The one spec feedback:** if BLAKE3 is **slower** than SHA-256 on this ARM core
  (e.g. the host has SHA hardware acceleration that BLAKE3 can't match), that revisits
  ADR-0015's *provisional* blob-digest default. Note the number either way.

Measure the **real on-disk cost per event** while the big log is loaded (feeds the
sizing appendix with a *measured* number, not an estimate):

```bash
psql "$CONN" -tAc "select pg_size_pretty(pg_total_relation_size('event_log')),
                          pg_total_relation_size('event_log')::float / count(*)
                   from event_log;"
```

---

## 8. The smaller board (the floor experiment)

Once the Pi 5 passes, repeat §5–§7 on the smaller candidate to find the actual floor.
Use a **distinct `--label`** and a separate `--json-out`, then compare the two records
side by side — the headroom multipliers shrink toward 1.0 as you approach the floor.

> [!NOTE]
> **The floor candidate is a Pi 4 / 8 GB.** It's the more interesting "can older,
> cheaper hardware still serve a clinic" test, and dropping down to it changes only the
> `--label`, not the procedure. (A Pi 3 B/B+ tops out at 1 GB RAM, so it would be
> memory-bound long before these compute gates bite — not a board for this benchmark.)

The deployment spec's stated rural-clinic floor is "Raspberry Pi 5 class"
([§8](../../docs/spec/deployment.md)); this experiment is how we learn whether that can
honestly be relaxed, and the `--json-out` records are the evidence.

---

## 9. Record the result

The `--json-out` files are self-describing (board, RAM, kernel, PG version, the device
PGDATA sat on, governor, throttle state, binary profile, thresholds, and every row), so
they are the durable artifact. When the run is done:

1. Paste the printed table + the environment header into **Spike 0001 §8** (a new
   "Bet B — results" subsection, mirroring the Bet A §8 write-up), and commit the
   `--json-out` next to it.
2. If Bet A's primitives are to be finally ratified, fold the **B4 ARM number** into the
   ADR-0015 follow-up (it removes the "provisional" caveat on the blob-digest line, or
   revisits it).
3. Update the spike status line and `docs/HANDOVER.md`'s build-prep pointer to reflect
   the go/no-go and which mitigation rung (if any) was needed.

---

## Cleanup

```bash
# Stop the benchmark cluster (data stays on the SSD unless you remove it):
sudo -u postgres /usr/lib/postgresql/18/bin/pg_ctl -D /mnt/ssd/pgdata stop
# Restore the everyday governor if you changed it:
sudo cpupower frequency-set -g ondemand
```

---

## Appendix — does 1 TB hold a clinic's life? (sizing sanity check)

Bet A measured the *wire* plane at **~494 B/event**
([§8 A5](../../docs/spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#8-bet-a--results-cape-york--dorrigo-2026-06-16--pass-and-one-real-bug-fixed)).
On disk an event costs more — the signed bytes plus the parsed JSONB body, the
plaintext legibility twin, indexes, the projection row, and WAL — call it low single-digit
KB/event amortized (the §7 query gives you the *measured* figure for this build). Even at
a conservative **~4 KB/event on disk, 1 TB ≈ 250 million events**; at ~2 KB, ~500 million.

A busy rural clinic generating, very roughly, thousands of clinical events per day would
take **centuries** to fill 1 TB. So for a single clinic node the SSD's **capacity is not
the constraint** — the bet Bet B tests is **CPU** (projection maintenance + verification
staying cheap), exactly as ADR-0001 frames it. The capacity number that *does* bite is
the national replicated-essential tier (~25 KB/person → ~2.5 TB for 100 M), which is a
different tier on different hardware
([ADR-0016](../../docs/spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md)),
not this clinic Pi. The 1 TB SSD here is about **fast, durable, non-SD storage and room
to run the big-log B1 tier**, not about running out of space.
