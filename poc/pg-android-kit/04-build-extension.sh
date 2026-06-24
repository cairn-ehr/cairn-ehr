#!/usr/bin/env bash
#
# Spike 0003 G3 — cross-compile the pgrx smoke extension for aarch64-linux-android.
#
# WHY this exists, for a junior reader:
#   This is the genuinely novel step: building a pgrx (Rust-in-Postgres, ADR-0002)
#   extension for Android's bionic ABI and loading it into the bionic Postgres from
#   steps 01–03. Two things make a normal `cargo pgrx package` impossible here:
#     (a) pgrx's build runs `pg_config` to find the server headers, but our prefix's
#         pg_config is an aarch64 ELF that cannot execute on the host — so we feed
#         pgrx a host shim (extension/pg_config-shim) that returns the same paths
#         pointing at the HOST copy of the staged prefix (where bindgen can read
#         the headers).
#     (b) pgrx generates the install SQL by dlopen()ing the built .so — impossible
#         for an ARM .so on the host. We hand-write control + SQL instead (they are
#         architecture-independent); see extension/cairn_smoke*.{control,sql}.
#   bindgen itself uses the NDK's libclang with --target set for the device ABI.
#
# OUTPUT: <workdir>/libcairn_smoke.so  (+ the control/sql are in extension/)
set -euo pipefail

WORK="${1:?usage: 04-build-extension.sh <workdir>  (same workdir as steps 01/02)}"
NDK="${ANDROID_NDK:-$HOME/Library/Android/sdk/ndk/29.0.14206865}"
API="${ANDROID_API:-28}"; BINDGEN_API="${BINDGEN_API:-24}"
HOST="darwin-x86_64"; [ "$(uname)" = Linux ] && HOST="linux-x86_64"
TC="$NDK/toolchains/llvm/prebuilt/$HOST"
KIT="$(cd "$(dirname "$0")" && pwd)"
PREFIX="$WORK/prefix"
[ -d "$PREFIX/include/postgresql/server" ] || { echo "no server headers in $PREFIX — run 01 first" >&2; exit 1; }
command -v cargo >/dev/null || { echo "cargo not found" >&2; exit 1; }
rustup target list --installed 2>/dev/null | grep -qx aarch64-linux-android || rustup target add aarch64-linux-android

export PGPREFIX="$PREFIX"
export PGRX_PG_CONFIG_PATH="$KIT/extension/pg_config-shim"
export LIBCLANG_PATH="$TC/lib"
# bindgen parses the PG server headers for the DEVICE ABI (not the host's):
export BINDGEN_EXTRA_CLANG_ARGS="--target=aarch64-linux-android${BINDGEN_API} --sysroot=$TC/sysroot -I$PREFIX/include/postgresql/server -I$PREFIX/include"
# cc-rs (pgrx C shim) + the final link both use the NDK clang as the driver:
export CC_aarch64_linux_android="$TC/bin/aarch64-linux-android${API}-clang"
export AR_aarch64_linux_android="$TC/bin/llvm-ar"
export CFLAGS_aarch64_linux_android="--target=aarch64-linux-android${API}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TC/bin/aarch64-linux-android${API}-clang"

( cd "$KIT/extension" && cargo build --release --target aarch64-linux-android )

so="$KIT/extension/target/aarch64-linux-android/release/libcairn_smoke.so"
cp "$so" "$WORK/libcairn_smoke.so"
echo "=== built $WORK/libcairn_smoke.so ==="
# Count (not grep -q) to avoid a pipefail/SIGPIPE false negative from early-exit.
magic=$("$TC/bin/llvm-readelf" --dyn-syms "$WORK/libcairn_smoke.so" 2>/dev/null | grep -cE 'Pg_magic_func' || true)
[ "$magic" -ge 1 ] && echo "Pg_magic_func present — looks like a valid PG module" || echo "WARN: no Pg_magic_func"
