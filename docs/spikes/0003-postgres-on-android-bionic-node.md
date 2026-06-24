# Spike 0003 — A Cairn node on an Android phone (native bionic PostgreSQL 18 + pgrx)

- **Status:** **Ran 2026-06-25 — G0–G3 PASS** (full PASS per §6) on the target handset. Native
  PostgreSQL 18.2 execs, `initdb`s a cluster, serves real SQL over TCP, **and a pgrx extension
  cross-compiled for `aarch64-linux-android` loads and runs (incl. SPI)** — no Termux userland, no
  root, no VM. The runnable kit now exists at [`poc/pg-android-kit/`](../../poc/pg-android-kit/). See
  §10 for the run log.
- **Date:** 2026-06-19 (proposed) · **2026-06-25** (run)
- **Target hardware:** RedMagic 11 Pro (Snapdragon 8 Elite Gen 5, 24 GB RAM, 1 TB, Android 16 /
  REDMAGIC OS 11). A representative high-end phone-as-leaf-node.
- **Validates:** the **fractal-topology** invariant (*one codebase at every tier; a node's role is
  configuration, not a different product*) at its hardest tier — a consumer phone;
  [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md) (PostgreSQL *is* the node, even here);
  [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md) (the pgrx escape hatch must
  actually compile and load on the node's architecture); and the
  [ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) boundary (how Postgres
  got onto the node is invisible above the event-core socket).

> [!NOTE]
> This is build-prep, not architecture. It does not propose changing the design; it asks whether the
> *existing* design's "any node, one codebase" claim survives contact with the most hostile substrate a
> node might ever run on — a non-rooted Android phone with a foreign libc and a locked-down hypervisor.

---

## 1. Why this spike, and why now

The fractal-topology claim is load-bearing for the anti-capture mission: a workstation, a Pi, a facility
server and a *phone in a clinician's pocket* are meant to be the same product in different
configuration. Spike 0001 exercises the Pi-class tier. The phone tier is harder and untested, and a real
prompt exists for it: a clinician-grade handset (RedMagic 11 Pro) that already runs a 4B model at 27 tok/s
on its NPU is exactly the kind of capable edge device a Cairn node might inhabit. The question is narrow
and answerable: **can a node's substrate — PostgreSQL 18 plus a Rust/pgrx safety-critical extension — run
natively on stock Android, with no Termux on the device, no root, and no Linux VM?**

"No Termux, no VM" is the point. Termux would make this trivial but is a third-party userland a clinic
can't depend on; a VM would be clean but — see §3 — is not available on this class of phone. The test is
whether Cairn's substrate runs as *first-class native code* on the device.

---

## 2. What this spike is *not*

- **Not** a proposal to ship a phone node, or to support Android as a tier. It is a feasibility probe.
- **Not** a performance bet (that is Spike 0001's Bet B). This is a *can-it-run-at-all* bet.
- **Not** an endorsement of the phone as a clinical endpoint. Possession semantics, display, and
  paper-parity for a phone form factor are out of scope here.

---

## 3. The first finding: the easy path is closed *by the silicon*, not the OS

The obvious route — Android's official **AVF Linux Terminal** (a pKVM-isolated Debian VM, since Android
15 QPR2 / 16, where `apt install postgresql-18` just works) — **does not work on this phone, and the
reason is the SoC.** Google's Terminal requires *non-protected* VMs (host can see guest memory). Current
Qualcomm Snapdragons, including the **8 Elite Gen 5** in the RedMagic 11 Pro, expose **protected VMs
only**: `ro.boot.hypervisor.protected_vm.supported = true` while `ro.boot.hypervisor.vm.supported` is
empty. This is platform-wide across current Qualcomm parts and is **not a REDMAGIC OS toggle**. MediaTek
Dimensity and Exynos 2500 devices *do* expose non-protected VMs and run the Terminal.

> [!IMPORTANT]
> **Procurement consequence for phone nodes:** the cheap, stock, fully-supported path (AVF VM → stock
> Postgres → stock pgrx) is available on **MediaTek/Exynos** handsets and **denied on Qualcomm
> flagships**. On Qualcomm the only native path is the bionic port below. This is a hardware-selection
> input, set by the SoC's hypervisor policy — nothing in Cairn's design controls it.

So on the target device, the universe collapses to: **(A) a native bionic build**, or (C) a rooted
daemon (same build, fewer sandbox constraints). The spike pursues (A).

---

## 4. The second finding: the bionic build is already solved and maintained

The native build is not green-field. **Termux ships PostgreSQL 18.2 for `aarch64` today**, and its patch
set is the authoritative recipe. The durable facts, lifted from that build rather than inferred:

| Obstacle (bionic vs glibc) | Resolution in the maintained build |
|---|---|
| No System V IPC | main shared memory via **`libandroid-shmem`** (SysV emulated on ashmem/memfd), linked `-landroid-shmem -llog` |
| Dynamic shared memory | **forced to `mmap`** — `choose_dsm_implementation()` returns `"mmap"` under `__ANDROID__` |
| No `sem_open` (named POSIX) | build with **`USE_UNNAMED_POSIX_SEMAPHORES=1`** |
| No `locale -a`, thin libc locales | `READ_LOCALE_A_OUTPUT` disabled; `initdb` hardcodes `en_US.UTF-8`/UTF-8; **collation via ICU** (`--with-icu`) |
| `initdb` "cannot locate symbol" | linker cache var `pgac_cv_prog_cc_LDFLAGS_EX_BE__Wl___export_dynamic=yes` (`--export-dynamic`) |
| tzdata hard links unsupported | host-built `zic` patched to prefer symlinks |
| No Termux prefix on a stock phone | **Postgres is relocatable** (derives paths from `argv[0]`) → run the tree from `/data/local/tmp` with `LD_LIBRARY_PATH` |

Configure essence (full reference flags carried in the kit):

```
--with-icu --with-libxml --with-openssl --with-uuid=e2fs
USE_UNNAMED_POSIX_SEMAPHORES=1
ZIC=<host-built zic>
pgac_cv_prog_cc_LDFLAGS_EX_BE__Wl___export_dynamic=yes
pgac_cv_prog_cc_LDFLAGS__Wl___as_needed=yes
```

**Build strategy:** drive `termux-packages`' Docker builder *on a laptop* to produce the `aarch64`
`postgresql` + runtime-dependency `.debs` (`libandroid-shmem`, `libicu`, `libxml2`, `openssl`,
`readline`, `zlib`, `libuuid`, `libandroid-execinfo`, `libc++`), then flatten them into one relocatable
prefix. Termux is used **only as a cross-compiler on the laptop**; nothing from it lands on the phone.

---

## 5. The unproven step: pgrx on bionic

PL/pgSQL + SQL + constraints — most of the [ADR-0021/0022](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)
enforcement floor — port for free with the Postgres build. The **safety-critical Rust core
([ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)) is the long pole**: nobody has
publicly cross-compiled a pgrx extension for `aarch64-linux-android` and loaded it into a bionic Postgres.
pgrx **0.18.1 has a `pg18` feature**, so version support is not the blocker; the cross-build is. Two
specific gotchas, both handled in the kit:

1. **`pgrx-pg-sys` runs `bindgen` against the *target* server headers.** Point
   `BINDGEN_EXTRA_CLANG_ARGS` at the termux-built server headers with
   `--target=aarch64-linux-android` so `pg_sys` matches bionic, not the host glibc.
2. **`cargo pgrx package` generates `.control`/`.sql` by `dlopen()`ing the built `.so`** — impossible for
   an ARM `.so` on an x86 host. Generate control+sql **once from a native build** (they are
   architecture-independent) and pair them with the cross-built cdylib.

The probe extension is deliberately tiny but touches the layers most likely to fault under a foreign
libc: a plain return, argument passing, `palloc`/varlena, and **SPI** (a call back into the executor).

---

## 6. PASS / FAIL

The single highest-information observation is the **first** one:

| Gate | Observation | Meaning |
|---|---|---|
| **G0 — exec** | `postgres --version` runs from `/data/local/tmp` as the `shell` user | If SELinux W^X denies it on REDMAGIC OS 11, a real node needs the **APK / `jniLibs` `nativeLibraryDir`** shape (binaries shipped as `lib*.so`) or root. That denial *is* the finding. |
| **G1 — initdb** | `initdb` completes a cluster | bionic locale/zic/export-dynamic chain holds |
| **G2 — start** | postmaster stays up, accepts a socket connection | `libandroid-shmem` works against Android 16's memfd/ashmem (**#1 risk on a brand-new OS**; fallback = a memfd-based shim such as `libwrapdroid`) |
| **G3 — pgrx** | `CREATE EXTENSION` + the four smoke functions return | the ADR-0002 escape hatch is viable on this tier |

**PASS** = G0–G3 all green. **PARTIAL** (G0–G2 only) still ratifies "Postgres-the-node runs natively on a
stock Qualcomm phone," with the pgrx core deferred. **FAIL at G0** redirects the phone tier to the
APK-embedded shape and is itself a useful architectural result.

---

## 7. Exit criteria → what this feeds back

- **G0–G2 PASS** → a build-prep note (not an ADR) recording that the phone tier is reachable natively,
  plus the **MediaTek/Exynos-vs-Qualcomm** procurement guidance from §3.
- **G3 PASS** → the first evidence that the [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)
  pgrx escape hatch holds at *every* tier, strengthening the fractal-topology claim.
- **Any FAIL** → a scoped question back to design: does the phone tier require the APK-embedded execution
  model, and if so, is "a node is configuration, not a different product" still literally true for phones,
  or does it become "the same *event core* in a different *package*"? That nuance, if forced, belongs in
  the topology discussion, not silently in a build script.

---

## 8. Blast-radius (§9) note

Nothing in this spike sits on the inter-node path. A phone node, however it was built, speaks the same
signed append-only **event core** over the same socket as any other node — the
[ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) boundary is *below* the
question this spike asks. The spike is therefore a stress-test **of** that boundary's promise (substrate
provenance is invisible upstream), not a change to it. The safety-critical members built here (the pgrx
extension) fall under the §9 "Rust / in-database, reviewer-legible" rule unchanged; cross-compilation
does not alter what code is allowed to be.

---

## 9. Reproduction

A runnable kit (`cairn-pg-android-kit`) accompanies this spike as a build-prep artifact: the maintained
termux PG-18 bionic patches, the pgrx 0.18.1 smoke extension, and four scripts —
build (termux-Docker cross-compile) → stage (flatten `.debs` to a relocatable prefix) → cross-compile the
extension → `adb push` + `initdb` + `CREATE EXTENSION` + `SELECT`. It runs on a laptop with the phone on
USB. The build deliberately reuses the maintained patch set rather than hand-rolling an NDK build, so the
only novel surface under test is §5.

> [!NOTE]
> The kit as actually built ([`poc/pg-android-kit/`](../../poc/pg-android-kit/)) took the faster route
> for G0–G2: it flattens Termux's **prebuilt** PG-18 `.debs` into the relocatable prefix rather than
> driving a from-source termux-Docker build. The only component compiled from source is the one that had
> to be (`libandroid-shmem`, see §10). The from-source PG build and the pgrx extension (§5) remain for G3.

---

## 10. Run log — 2026-06-25 (G0–G2 PASS)

Run on the target handset (`NX809J` / nubia RedMagic 11 Pro, `ro.soc.model=SM8850`, Android 16 / SDK 36,
`arm64-v8a`, SELinux **enforcing**) over `adb`, as the `shell` user from `/data/local/tmp`.

| Gate | Observation |
|---|---|
| **§3** | Confirmed on-device: `protected_vm.supported=true`, `vm.supported=` (empty), hypervisor `gunyah`. The stock AVF-VM path is denied on this Qualcomm part — bionic is the only native route, as desk-checked. |
| **G0 — exec** | ✅ A bionic aarch64 PIE pushed to `/data/local/tmp` execs as `shell` under enforcing SELinux. The feared W^X denial does not bite for the adb/`shell` domain. |
| **G1 — initdb** | ✅ Cluster initialized: `dynamic shared memory implementation … mmap` (as §4 predicted), ICU collation, data-page checksums on. |
| **G2 — start** | ✅ Postmaster up; TCP bind on `127.0.0.1` succeeds; `psql` over TCP runs `version()` + `CREATE/INSERT/SELECT`. |
| **G3 — pgrx** | ✅ A pgrx 0.18.1 smoke extension cross-built for `aarch64-linux-android` `CREATE EXTENSION`s and its four probes return — plain `42`, `add(40,2)=42`, varlena `echo→'cairn:phone-node'`, and **SPI** `count=3`. The [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md) escape hatch holds at the phone tier. |

**The blocker, and its fix.** §4 named `libandroid-shmem` "the #1 risk." It was — but not for the
predicted Android-16 ashmem/memfd reason. The prebuilt Termux lib has **two** defects on a stock device
(both isolated by `strace`), fixed by recompiling that one ~600-line BSD lib with the NDK:

1. **Infinite CPU spin** — it coordinates SysV-shm keys via symlinks under a *compile-time-baked Termux
   prefix* (`/data/data/com.termux/files/usr/tmp`), unwritable here → `EACCES` → ~millions of retry
   iterations, never the expected `EEXIST`. Fixed by deriving the dir from `$TMPDIR` at runtime.
2. **`/dev/ashmem` removed (Android 11+)** — Termux builds at API 24 → legacy `/dev/ashmem` path →
   `EACCES`. Building at API ≥ 26 uses `ASharedMemory_create`, but that drags in `libandroid.so` →
   the handset's vendor `libvendorutils.so`, whose `BIO_flush` clashes with the bundled OpenSSL. Fixed
   by backing regions with a plain `memfd_create()` syscall — the `.so` then needs only
   `liblog`/`libdl`/`libc`.

**Operational note** (not a failure): Unix-domain sockets cannot be created under `/data/local/tmp`
(SELinux denies a socket node in `shell_data_file`); TCP loopback works, so the node runs TCP-only.

**G3 cross-compile (the §5 long pole).** Contrary to the desk-check, **bindgen against the bionic
server headers was a non-event** — fed a host `pg_config` *shim* (the real one is an aarch64 ELF that
won't run on the host) plus `BINDGEN_EXTRA_CLANG_ARGS=--target=aarch64-linux-android… --sysroot=<NDK>`,
it generated `pg18.rs` first try. The only friction: `cc-rs` (pgrx's C shim) wants
`CC_aarch64_linux_android` pointed at the NDK's *versioned* clang, and §5 gotcha #2 held — `cargo pgrx
schema`/`package` is impossible (it `dlopen()`s the ARM `.so`), so the control + install SQL are
hand-written (architecture-independent). The cross-built `.so` needs only `libdl`/`libc`; PG symbols
resolve at `LOAD` time inside the postmaster.

**What this feeds back (per §7):** the phone tier is reachable natively on a stock Qualcomm flagship
across **all four gates**, with the MediaTek/Exynos-vs-Qualcomm procurement guidance from §3 confirmed
on real hardware. The fractal-topology claim and the [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)
pgrx escape hatch both **survive contact with the most hostile substrate a node might run on**. The
remaining honest gaps are packaging (APK/`jniLibs` shape for a non-`shell`-user node) and a from-source
PG build, neither of which is load-bearing for the feasibility question this spike asked.
