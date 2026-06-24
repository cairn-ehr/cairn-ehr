#!/usr/bin/env bash
#
# Spike 0003 G3 — install the cross-built pgrx extension on the device and run it.
#
# Assumes steps 01–04 have run (the bionic Postgres prefix is on the device from
# step 03, and <workdir>/libcairn_smoke.so exists from step 04). Installs the
# module + control + SQL into the on-device prefix, starts the postmaster TCP-only,
# CREATE EXTENSIONs it, and calls the four smoke functions.
set -euo pipefail

WORK="${1:?usage: 05-run-extension-on-device.sh <workdir> [adb-serial]}"
SERIAL="${2:-}"
ADB="${ADB:-$HOME/Library/Android/sdk/platform-tools/adb}"
KIT="$(cd "$(dirname "$0")" && pwd)"
DEV=/data/local/tmp/cairnpg; PORT="${PG_PORT:-5432}"
adb() { if [ -n "$SERIAL" ]; then "$ADB" -s "$SERIAL" "$@"; else "$ADB" "$@"; fi; }

[ -f "$WORK/libcairn_smoke.so" ] || { echo "no $WORK/libcairn_smoke.so — run step 04 first" >&2; exit 1; }
adb shell "[ -x $DEV/bin/postgres ]" || { echo "Postgres not on device — run step 03 first" >&2; exit 1; }

echo "=== install extension into the on-device prefix ==="
adb push "$WORK/libcairn_smoke.so"            "$DEV/lib/postgresql/cairn_smoke.so"           >/dev/null
adb push "$KIT/extension/cairn_smoke.control" "$DEV/share/postgresql/extension/"             >/dev/null
adb push "$KIT/extension/cairn_smoke--0.0.0.sql" "$DEV/share/postgresql/extension/"          >/dev/null

echo "=== G3: CREATE EXTENSION + the four smoke probes ==="
adb shell "
  BASE=$DEV
  export LD_LIBRARY_PATH=\$BASE/lib PATH=\$BASE/bin:\$PATH HOME=/data/local/tmp TZ=UTC TMPDIR=\$BASE/tmp PGDATA=\$BASE/data
  rm -f \$BASE/tmp/ashv_key_*
  postgres -D \$BASE/data -c unix_socket_directories= -c listen_addresses=127.0.0.1 -c port=$PORT >\$BASE/g3.log 2>&1 &
  sleep 4
  PSQL=\"psql -h 127.0.0.1 -p $PORT -U postgres -d postgres -tA\"
  \$PSQL -c 'DROP EXTENSION IF EXISTS cairn_smoke;'
  \$PSQL -c 'CREATE EXTENSION cairn_smoke;'
  echo '(1) plain return  :' \$(\$PSQL -c 'select cairn_smoke_answer();')
  echo '(2) arg passing   :' \$(\$PSQL -c 'select cairn_smoke_add(40,2);')
  echo '(3) varlena/palloc :' \$(\$PSQL -c \"select cairn_smoke_echo('phone-node');\")
  echo '(4) SPI           :' \$(\$PSQL -c 'select cairn_smoke_spi();')
  pg_ctl -D \$BASE/data stop -m fast >/dev/null 2>&1
" | tr -d '\r'

echo
echo "PASS if: CREATE EXTENSION succeeded and the four probes printed 42 / 42 / cairn:phone-node / 3."
