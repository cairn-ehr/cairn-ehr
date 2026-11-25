# Cairn terminal packaging (#909 slice C2)

What a Raspberry Pi terminal needs so that its window follows its node: a root-owned installer
started by a systemd path unit, and a kiosk unit that runs the window unprivileged.

| File | Role |
|------|------|
| `cairn-terminal-install.path` | Starts the installer when `incoming/complete` appears. |
| `cairn-terminal-install.service` | The installer, as root, once per trigger (`KillMode=control-group`). |
| `cairn-window-kiosk.service` | `cage -- cairn-window …` as user `cairn-window`, never root. |
| `install.sh` | The first hand install (the terminal's genesis). POSIX sh, idempotent. |

## Layout

| Path | Owner / mode |
|------|--------------|
| `/opt/cairn/bin` (program, installer) | root, 0755 |
| `/etc/cairn` (`channel-root.cbor`, `window.env`) | root, 0755 |
| `/var/lib/cairn-terminal/incoming` | root:cairn-window, 2770 (the window stages here) |
| `/var/lib/cairn-terminal/state` | root, 0755 (pin, installed release, `status.json` 0644) |
| `/var/lib/cairn-window/terminal` | cairn-window, 0700 (the window's key and node pin) |

## First install

On a Raspberry Pi OS Lite 64-bit (Debian trixie) terminal, with the two binaries from a
steward-signed release's CI build and the channel's first trust file:

    sudo ./install.sh --window cairn-window --installer cairn-terminal-install \
        --trust channel-root.cbor --node-fingerprint <hex from the node's gateway-setup>

The script prints the trust file's SHA-256. **Check it against a second channel** (the
fingerprint the Cairn project published elsewhere) before trusting the terminal: that file is
the root of everything the terminal will ever accept. An existing, different trust file is never
overwritten; the script says so and leaves it.

The connection string, if the node is not on this machine's default socket, is passed with
`--conn` and stored in `/etc/cairn/window.env` (root:cairn-window, 0640).

## Requirements and notes

* `fs.protected_hardlinks=1` (Debian's default) is **required**: the installer relies on it
  when it reads the staging folder the window can write.
* `cage` is the kiosk compositor, installed from apt, not vendored. Licence: **MIT**
  (checked against upstream's `LICENSE`; AGPL-3.0-compatible).
* The units and script have not yet been run on a real Pi (the RUNBOOK's Linux check is owed).

## The release build

`.github/workflows/release-build.yml` builds the unsigned aarch64 artifacts on Debian trixie
and uploads them with a `window.json` and SHA-256 sums. It holds no key: the steward signs the
manifest offline.
