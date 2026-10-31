#!/usr/bin/env python3
# scripts/macos-patch-pgx-dylib.py — a macOS-ONLY local-dev workaround, never used by CI.
#
# The 2026-10-19 Xcode linker (ld) emits libcairn_pgx.dylib with its LINKEDIT string table
# mis-aligned, so Postgres cannot load the extension on a Mac rebuilt with that Xcode. This script
# moves the string table 4 bytes to fix the alignment, and REFUSES (exits non-zero, writes nothing)
# unless the dylib has exactly that defect: a stroff that is 4 mod 8 and 4 bytes of zero padding
# after the table. Linux builds and CI never need it. Drop it once a fixed Xcode ships.
#
# Workaround for the Oct-2026 Xcode ld emitting a dylib whose LINKEDIT string pool is
# mis-aligned (LC_SYMTAB.stroff % 8 == 4), which dlopen/Postgres refuse.
#
# Full local reinstall sequence (Mac, pg18, run from inside extensions/cairn_pgx/):
#   cd extensions/cairn_pgx; unset CARGO_TARGET_DIR
#   export BINDGEN_EXTRA_CLANG_ARGS="-isysroot $(xcrun --show-sdk-path)"
#   export RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup -C link-arg=-Wl,-no_fixup_chains"
#   cargo pgrx install --release --pg-config "/Applications/Postgres 2.app/Contents/Versions/18/bin/pg_config"
#   python3 <this script> target/release/libcairn_pgx.dylib
#   codesign -f -s - target/release/libcairn_pgx.dylib
#   cp target/release/libcairn_pgx.dylib "/Applications/Postgres 2.app/Contents/Versions/18/lib/postgresql/cairn_pgx.dylib"
#   for d in cairn_test cairn_test2 cairn_test3; do
#     psql "host=127.0.0.1 port=5532 user=hherb dbname=$d" -c 'DROP EXTENSION cairn_pgx CASCADE; CREATE EXTENSION cairn_pgx;'
#   done
# Usage: python3 scripts/macos-patch-pgx-dylib.py <dylib>
import struct, sys

def main(path):
    b = bytearray(open(path, "rb").read())
    if len(b) < 32 or struct.unpack_from("<I", b, 0)[0] != 0xFEEDFACF:  # MH_MAGIC_64
        sys.exit("refuse: not a thin 64-bit Mach-O file")
    ncmds, = struct.unpack_from("<I", b, 16)  # mach_header_64.ncmds
    off = 32                                   # first load command
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", b, off)
        if cmd == 2:  # LC_SYMTAB
            _, _, stroff, strsize = struct.unpack_from("<IIII", b, off + 8)
            if stroff % 8 != 4:
                sys.exit(f"refuse: stroff={stroff} (% 8 = {stroff % 8}), not the known 4-mod-8 defect")
            tail = bytes(b[stroff + strsize : stroff + strsize + 4])
            if tail != b"\0\0\0\0":
                sys.exit(f"refuse: 4 bytes after the string table are not zero padding: {tail.hex()}")
            b[stroff + 4 : stroff + 4 + strsize] = b[stroff : stroff + strsize]
            b[stroff : stroff + 4] = b"\0\0\0\0"
            struct.pack_into("<I", b, off + 16, stroff + 4)
            open(path, "wb").write(b)
            print(f"patched: stroff {stroff} -> {stroff + 4}, strsize {strsize}; now run codesign -f -s -")
            return
        off += size
    sys.exit("refuse: no LC_SYMTAB found")

if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: macos-patch-pgx-dylib.py <dylib>")
    main(sys.argv[1])
