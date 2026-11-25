#!/bin/sh
# First hand install of a Cairn terminal (#909 slice C2): the terminal's genesis. After this the
# terminal follows its node by itself. Safe to re-run.
#
# Usage: install.sh --window <cairn-window> --installer <cairn-terminal-install> \
#                    --trust <channel-root.cbor> [--node-fingerprint <hex>] [--conn <string>]
set -eu

[ "$(id -u)" -eq 0 ] || { echo "install.sh: run as root" >&2; exit 1; }

window=""; installer=""; trust=""; fingerprint=""; conn=""
while [ $# -gt 0 ]; do
    case "$1" in
        --window) window=$2 ;;
        --installer) installer=$2 ;;
        --trust) trust=$2 ;;
        --node-fingerprint) fingerprint=$2 ;;
        --conn) conn=$2 ;;
        *) echo "install.sh: unknown argument $1" >&2; exit 1 ;;
    esac
    shift 2 || { echo "install.sh: $1 needs a value" >&2; exit 1; }
done
[ -f "$window" ] && [ -f "$installer" ] && [ -f "$trust" ] || {
    echo "install.sh: --window, --installer and --trust must name existing files" >&2; exit 1; }

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

install -o root -g root -m 0755 "$window" /opt/cairn/bin/cairn-window
install -o root -g root -m 0755 "$installer" /opt/cairn/bin/cairn-terminal-install

# The trust file is the root of everything this terminal will ever accept: never replace one
# silently.
if [ -e /etc/cairn/channel-root.cbor ] && ! cmp -s "$trust" /etc/cairn/channel-root.cbor; then
    echo "install.sh: /etc/cairn/channel-root.cbor already exists and DIFFERS; leaving it alone." >&2
    echo "  Remove it by hand if you really mean to change channel." >&2
else
    install -o root -g root -m 0644 "$trust" /etc/cairn/channel-root.cbor
fi
echo "Trust file SHA-256 (compare with the second channel before use):"
sha256sum /etc/cairn/channel-root.cbor

# Window environment: root-written, readable by cairn-window only (it may hold a password).
umask 077
{
    [ -z "$conn" ] || printf 'CAIRN_CONN=%s\n' "$conn"
    if [ -n "$fingerprint" ]; then printf 'WINDOW_ARGS=--node-fingerprint %s\n' "$fingerprint"
    else printf 'WINDOW_ARGS=\n'; fi
} > /etc/cairn/window.env
chown root:cairn-window /etc/cairn/window.env
chmod 0640 /etc/cairn/window.env

install -m 0644 "$here/cairn-terminal-install.service" "$here/cairn-terminal-install.path" \
    "$here/cairn-window-kiosk.service" /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now cairn-terminal-install.path
systemctl enable cairn-window-kiosk.service
echo "Installed. Reboot to start the kiosk."
