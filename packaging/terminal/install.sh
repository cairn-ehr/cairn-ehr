#!/bin/sh
# First hand install of a Cairn terminal (#909 slice C2): the terminal's genesis. After this the
# terminal follows its node by itself. Safe to re-run.
#
# Usage: install.sh --window <cairn-window> --installer <cairn-terminal-install> \
#                    --trust <channel-root.cbor> --node-fingerprint <hex> --conn <string> \
#                    [--replace-window]
#
# A terminal is REMOTE by definition: its window reaches the node through the node's gateway
# (--terminal-dir), and a window with a local database refuses --terminal-dir. So a first
# install needs both --node-fingerprint and --conn. A re-run may omit both (the existing
# /etc/cairn/window.env is kept); with both, it is rewritten.
# The connection string may carry a password: prefer the CAIRN_WINDOW_CONN environment variable
# to --conn, which shows in `ps` and shell history.
set -eu

die() { echo "install.sh: $*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run as root"

window=""; installer=""; trust=""; fingerprint=""; conn="${CAIRN_WINDOW_CONN:-}"; replace=no
while [ $# -gt 0 ]; do
    case "$1" in
        --replace-window) replace=yes; shift; continue ;;
        --window|--installer|--trust|--node-fingerprint|--conn) ;;
        *) die "unknown argument $1" ;;
    esac
    [ $# -ge 2 ] || die "$1 needs a value"
    case "$1" in
        --window) window=$2 ;;
        --installer) installer=$2 ;;
        --trust) trust=$2 ;;
        --node-fingerprint) fingerprint=$2 ;;
        --conn) conn=$2 ;;
    esac
    shift 2
done
[ -f "$window" ] && [ -f "$installer" ] && [ -f "$trust" ] ||
    die "--window, --installer and --trust must name existing files"

# Decide the window's environment BEFORE touching the system.
if [ -n "$conn" ] && [ -n "$fingerprint" ]; then
    write_env=yes
elif [ -n "$conn" ] || [ -n "$fingerprint" ]; then
    die "give both --conn and --node-fingerprint, or neither (on a re-run)"
elif [ -e /etc/cairn/window.env ]; then
    write_env=no
else
    die "a first install needs both --conn and --node-fingerprint (a terminal reaches its node remotely)"
fi
# systemd's EnvironmentFile strips backslashes and quotes from an unquoted value, so the string
# is written double-quoted with systemd's four escapes (" \ ` $); a newline cannot be written.
case "$conn" in *"
"*) die "the connection string must be one line" ;; esac
conn_quoted=$(printf '%s' "$conn" | sed 's/[\\"`$]/\\&/g')

here=$(cd "$(dirname "$0")" && pwd)

# cage (MIT) is the kiosk compositor; from apt, not vendored.
apt-get update
apt-get install -y --no-install-recommends cage libwebkit2gtk-4.1-0

id cairn-window >/dev/null 2>&1 || useradd --system --create-home --home-dir /var/lib/cairn-window \
    --shell /usr/sbin/nologin --groups video,input,render cairn-window

install -d -o root -g root -m 0755 /opt/cairn /opt/cairn/bin /etc/cairn /var/lib/cairn-terminal
install -d -o root -g cairn-window -m 2770 /var/lib/cairn-terminal/incoming
install -d -o root -g root -m 0755 /var/lib/cairn-terminal/state
install -d -o cairn-window -g cairn-window -m 0700 /var/lib/cairn-window/terminal

# The installer owns window updates from now on: a re-run must not downgrade the program.
if [ -e /opt/cairn/bin/cairn-window ] && [ "$replace" = no ]; then
    echo "install.sh: /opt/cairn/bin/cairn-window exists; keeping it (use --replace-window to replace)." >&2
else
    install -o root -g root -m 0755 "$window" /opt/cairn/bin/cairn-window
fi
install -o root -g root -m 0755 "$installer" /opt/cairn/bin/cairn-terminal-install

# Ruling R29 (rollback): the installer refuses any release not newer than
# state/installed_release. A fresh terminal has none, so a compromised window could stage an
# OLDER genuine signed release and root would install it. So record the program's own release
# first, whenever the file is absent or the program was just replaced by hand (a deliberate
# downgrade by the person at the terminal is theirs to make). The program is asked as
# cairn-window, never as root; a dev build answers 0, which every signed release is newer than.
if [ ! -e /var/lib/cairn-terminal/state/installed_release ] || [ "$replace" = yes ]; then
    release=$(runuser -u cairn-window -- /opt/cairn/bin/cairn-window --print-release-version) ||
        die "/opt/cairn/bin/cairn-window did not answer --print-release-version"
    case "$release" in
        ''|*[!0-9]*) die "/opt/cairn/bin/cairn-window gave \"$release\" as its release, not a number" ;;
    esac
    tmp=$(mktemp)
    printf '%s' "$release" > "$tmp"
    install -o root -g root -m 0644 "$tmp" /var/lib/cairn-terminal/state/installed_release
    rm -f "$tmp"
    echo "install.sh: recorded the installed window as release $release."
fi

# The trust file is the root of everything this terminal will ever accept: never replace one
# silently.
if [ -e /etc/cairn/channel-root.cbor ] && ! cmp -s "$trust" /etc/cairn/channel-root.cbor; then
    echo "install.sh: /etc/cairn/channel-root.cbor already exists and DIFFERS; leaving it alone." >&2
    echo "  To really change channel, remove it AND /var/lib/cairn-terminal/state/roots.cbor (which" >&2
    echo "  wins over it once it exists) and state/status.json by hand, then re-run." >&2
else
    install -o root -g root -m 0644 "$trust" /etc/cairn/channel-root.cbor
fi
echo "Trust file SHA-256 (compare with the second channel before use):"
sha256sum /etc/cairn/channel-root.cbor

# Window environment: root-written, readable by cairn-window only (it may hold a password).
if [ "$write_env" = yes ]; then
    (
        umask 077
        printf 'CAIRN_CONN="%s"\n' "$conn_quoted" > /etc/cairn/window.env
        printf 'WINDOW_ARGS=--node-fingerprint %s\n' "$fingerprint" >> /etc/cairn/window.env
    )
    chown root:cairn-window /etc/cairn/window.env
    chmod 0640 /etc/cairn/window.env
fi

install -m 0644 "$here/cairn-terminal-install.service" "$here/cairn-terminal-install.path" \
    "$here/cairn-window-kiosk.service" /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now cairn-terminal-install.path
# The window fetches nothing until the installer's state/status.json exists, and the path unit
# starts the installer only once a release is staged. So start it once now: with nothing staged
# it writes the idle status carrying this terminal's pin. Through systemd (same defaults and
# hardening), so it never runs beside a run the path unit started.
# Ruling R29: the run must leave "state": "idle". A trust file the installer cannot read makes it
# write a refused (terminal's own fault) status and fail instead, and that must stop the install
# here rather than end in "Installed.". The old status is removed first so that this run writes
# a fresh one (an existing status is never overwritten by an idle run); all it forgets is a
# release refused for good, which the window may then fetch once more and see refused again.
rm -f /var/lib/cairn-terminal/state/status.json
systemctl start cairn-terminal-install.service ||
    echo "install.sh: the installer's first run failed; see journalctl -u cairn-terminal-install." >&2
grep -qs '"state": "idle"' /var/lib/cairn-terminal/state/status.json ||
    die "the installer did not report idle in /var/lib/cairn-terminal/state/status.json. If it says a fault on the terminal itself, the trust file /etc/cairn/channel-root.cbor (or state/roots.cbor) is not one it can read: install the channel's real trust file. If it found a release staged, re-run. See journalctl -u cairn-terminal-install."
systemctl enable cairn-window-kiosk.service
echo "Installed. Reboot to start the kiosk."
