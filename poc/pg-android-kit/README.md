# cairn-pg-android-kit

Build-prep kit for **[Spike 0003](../../docs/spikes/0003-postgres-on-android-bionic-node.md)** —
running a Cairn node's substrate (native PostgreSQL 18) on a **stock Android phone**: no Termux
userland, no root, no Linux VM.

**Result on the target handset (RedMagic 11 Pro / Snapdragon 8 Elite Gen 5 / Android 16):
G0–G3 PASS.** PostgreSQL 18.2 runs as a native bionic process, `initdb` builds a cluster, the
postmaster serves real SQL over TCP, **and a pgrx (Rust-in-Postgres) extension cross-compiled for
`aarch64-linux-android` loads and runs — including SPI.** Every gate the spike defined is green.

This is a feasibility probe, not a product. See the spike doc for scope and the architectural "why".

---

## What's here

| File | Role |
|---|---|
| `01-stage-prefix.sh` | Download Termux's prebuilt PG-18 `.debs` (+ dependency closure) and flatten them into one relocatable `prefix/` tree. Termux is used only as a *source of cross-compiled aarch64 binaries*; none of its userland lands on the phone. |
| `02-build-shmem.sh` | Fetch upstream `libandroid-shmem` (3-clause BSD), apply our portability patch, and cross-compile a fixed `libandroid-shmem.so` (NDK) that overwrites the broken prebuilt one. |
| `patches/libandroid-shmem-bionic-portability.patch` | Our two-part fix to `libandroid-shmem` (see below). |
| `03-run-on-device.sh` | Push the prefix to a connected device and run the G0/G1/G2 gates. |
| `extension/` | A tiny **pgrx** smoke extension (`cairn_smoke`): plain return, arg passing, varlena/palloc, SPI — plus a host `pg_config` shim and hand-written control+SQL. |
| `04-build-extension.sh` | Cross-compile the extension for `aarch64-linux-android` (the G3 long pole). |
| `05-run-extension-on-device.sh` | Install it on the device, `CREATE EXTENSION`, run the four probes (G3). |

## Prerequisites

- macOS or Linux host with the **Android NDK** (set `ANDROID_NDK`; defaults to the macOS SDK path)
  and **platform-tools** (`adb`; set `ADB` if not on `PATH`).
- `curl` and `bsdtar` (libarchive) on the host.
- A phone connected with **USB debugging** enabled. Find its serial with `adb devices`.

## Run it

```bash
WORK=/tmp/pg-android            # any scratch dir
SERIAL=XXXXXXXX                 # from `adb devices`

./01-stage-prefix.sh             "$WORK"
./02-build-shmem.sh              "$WORK"   # MUST run after 01 — overwrites the broken shmem lib
./03-run-on-device.sh            "$WORK" "$SERIAL"   # G0/G1/G2
./04-build-extension.sh          "$WORK"             # G3 build (needs cargo + the rust android target)
./05-run-extension-on-device.sh  "$WORK" "$SERIAL"   # G3 on device
```

`03` prints the version, the `initdb` result, and a `CREATE/INSERT/SELECT` smoke test (G0–G2).
`05` prints `CREATE EXTENSION` and the four pgrx probes — PASS = `42 / 42 / cairn:phone-node / 3` (G3).
The install is left at `/data/local/tmp/cairnpg` (server stopped); remove it with
`adb shell rm -rf /data/local/tmp/cairnpg`.

`04` needs a Rust toolchain with the `aarch64-linux-android` target (`rustup target add
aarch64-linux-android`) and **pgrx 0.18.1** (`cargo install cargo-pgrx --version 0.18.1`, though the
build itself uses the `pgrx` crate, not the CLI).

---

## The findings that shaped the kit

### §3 procurement fact (confirmed on-device)
`ro.boot.hypervisor.protected_vm.supported = true` while `ro.boot.hypervisor.vm.supported` is empty
(Gunyah hypervisor). So Android's stock **AVF Linux Terminal** path (a non-protected Debian VM where
`apt install postgresql` just works) is **denied on this Qualcomm flagship**. The native bionic build
in this kit is the only route. MediaTek/Exynos handsets expose non-protected VMs and can use the easy
path instead. Nothing in Cairn controls this; it is a hardware-selection input.

### The one real blocker: `libandroid-shmem` (and its two-part fix)
PostgreSQL needs System V shared memory; bionic has none, so Termux's `libandroid-shmem` emulates it.
The prebuilt lib has **two** independent defects on a stock (non-Termux) device, both found by `strace`:

1. **Infinite CPU spin.** It coordinates SysV-shm keys with symlinks under a path baked at compile
   time to the Termux prefix (`/data/data/com.termux/files/usr/tmp`). On a stock device that path is
   unwritable → `EACCES` → it busy-loops forever (observed: millions of `symlinkat`/`readlinkat`
   retries). **Fix:** derive the directory from `$TMPDIR` at runtime, with a writable fallback.
2. **`/dev/ashmem` is gone (Android 11+).** Termux builds at API 24, so the lib backs regions with the
   legacy `/dev/ashmem` device, which returns `EACCES` on Android 16. Building at API ≥ 26 switches to
   `ASharedMemory_create`, but that links `libandroid.so`, which on this handset pulls a vendor lib
   (`libvendorutils.so`) whose `BIO_flush` clashes with PostgreSQL's bundled OpenSSL. **Fix:** back
   regions with a plain `memfd_create()` syscall — no `libandroid`, no vendor chain. The resulting
   `.so` needs only `liblog`/`libdl`/`libc`, which are always present.

### Operational note: no Unix-domain sockets
Creating a Unix-domain socket under `/data/local/tmp` is **SELinux-denied** (a socket node in the
`shell_data_file` context). TCP loopback binds fine, so the kit runs **TCP-only**
(`unix_socket_directories=` empty, `listen_addresses=127.0.0.1`).

### G3 — the pgrx cross-compile (the "long pole")
Cross-compiling a pgrx extension for `aarch64-linux-android` and loading it into bionic Postgres was
the genuinely unproven step. Two practical notes from doing it:
- **bindgen against the bionic server headers "just worked"** once fed the right paths — the feared
  §5 gotcha #1 was a non-event. The keys: a host `pg_config` *shim* (the real one is an aarch64 ELF
  that can't run on the host) returning host-prefix paths, plus
  `BINDGEN_EXTRA_CLANG_ARGS=--target=aarch64-linux-android… --sysroot=<NDK>` so bindgen parses the
  headers for the device ABI. `cc-rs` (pgrx's C shim) needs `CC_aarch64_linux_android` pointed at the
  NDK's versioned clang (`aarch64-linux-androidNN-clang`; it looks for an unversioned name otherwise).
- **`cargo pgrx schema`/`package` is impossible here** (it `dlopen()`s the built `.so`, which is ARM)
  — §5 gotcha #2. The control file + install SQL are architecture-independent, so we **hand-write**
  them (`extension/cairn_smoke.{control,--0.0.0.sql}`), binding each function to its pgrx `*_wrapper`
  symbol via `LANGUAGE c`.

---

## Honest limits

- Uses Termux's **prebuilt** PG binaries (the fast route to G1/G2), not a from-source build we drive
  ourselves. Legitimate for a feasibility probe; the libs that mattered (`libandroid-shmem` for G2 and
  the whole pgrx extension for G3) *are* built from source here.
- The G3 extension is a **smoke test** — four trivial functions exercising the call ABI, palloc, and
  SPI. It proves the pgrx escape hatch loads and runs on this tier; it is not the real safety-critical
  extension.
- Runs as the `shell` user from `/data/local/tmp`. A real phone node would need the APK/`jniLibs`
  packaging shape; that is out of scope here (and G0 passing means it is not forced).

## Licensing
`libandroid-shmem` is 3-clause BSD (AGPL-compatible). We redistribute only our patch and fetch the
upstream source at build time; the upstream `LICENSE` is downloaded alongside it into the work dir.
`pgrx` is MIT (AGPL-compatible); the `extension/` crate itself is part of Cairn and is AGPL-3.0.
