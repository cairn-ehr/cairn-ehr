# ADR-0112 — A terminal follows its node

- **Status:** Accepted (#909 slice C2: the owner takes one steward-signed release into the node, and each Linux terminal
  fetches its window program through the pinned gateway, re-verifies it against its own pin and swaps it in by itself)
- **Date:** 2026-11-25
- **Spec version at acceptance:** 0.114
- **Issues:** [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909); design and plan PR
  [#1180](https://github.com/cairn-ehr/cairn-ehr/pull/1180), build PR
  [#1181](https://github.com/cairn-ehr/cairn-ehr/pull/1181). Design (maintainer-approved 2026-11-25):
  `docs/superpowers/specs/2026-11-25-909-slice-c2-signed-release-design.md` (D1–D6); plan:
  `docs/superpowers/plans/2026-11-25-909-slice-c2-signed-release.md` (its "Plan-time calls"), amended by the build's
  rulings. Filed by the build: [#1182](https://github.com/cairn-ehr/cairn-ehr/issues/1182),
  [#1183](https://github.com/cairn-ehr/cairn-ehr/issues/1183),
  [#1184](https://github.com/cairn-ehr/cairn-ehr/issues/1184),
  [#1185](https://github.com/cairn-ehr/cairn-ehr/issues/1185),
  [#1186](https://github.com/cairn-ehr/cairn-ehr/issues/1186),
  [#1187](https://github.com/cairn-ehr/cairn-ehr/issues/1187),
  [#1188](https://github.com/cairn-ehr/cairn-ehr/issues/1188),
  [#1189](https://github.com/cairn-ehr/cairn-ehr/issues/1189),
  [#1190](https://github.com/cairn-ehr/cairn-ehr/issues/1190).
- **Supersedes:** nothing, and it amends no ADR by edit.
- **Builds on:** [ADR-0055](0055-distribution-trust-root-governance-chained-root-document.md) (the chained trust-root
  document, the self-contained bundle, the gate order and its practice-manager refusals: this slice builds the pure core
  of its verifier, issue [#257](https://github.com/cairn-ehr/cairn-ehr/issues/257)'s, for window releases) ·
  [ADR-0110](0110-the-pinned-terminal-gateway.md) (the bytes ride the pinned link a remote terminal already has) ·
  [ADR-0111](0111-an-older-window-works-above-a-floor.md) (the 60 s poll, `install_fit`, and the restart that picks the
  new program up) · [ADR-0040](0040-signing-context-domain-separation.md) (the closed, add-only signing-context
  registry) · [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md) (COSE_Sign1 + Ed25519, canonical
  CBOR) · [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (a new `db/086`, `SCHEMA_GENERATION` 86)
- **Code:** `crates/cairn-event/src/release_sig.rs` (and the two contexts in `lib.rs`); `crates/cairn-release/`
  (`src/{docs,refusal,verify,steward,status,staging}.rs`, `src/bin/cairn-release.rs`, `official/README.md`);
  `db/086_release_store.sql`, `db/window_door_surface.txt`;
  `crates/cairn-node/src/{release_intake,window_floor,main}.rs`;
  `cairn-gui/cairn-gui-tauri/src/{update,release_fetch,main,state}.rs`, `src-ui/update.js`;
  `crates/cairn-terminal-install/`; `packaging/terminal/`; `.github/workflows/release-build.yml`

## Context

After ADR-0111 an upgraded node no longer disturbs its terminals, but each terminal still needs someone to install the
new window program on it by hand; until they do, a terminal below the floor finishes its chart and then does nothing
useful. The maintainer's D4 of slice C (2026-11-24): the owner upgrades the node **once**, and every terminal follows by
itself.

The danger in "follows by itself" is that the program arriving over the network is the one every clinician types their
passphrase into. A terminal that accepted whatever its node served would let one broken-into node take every clinician's
signing key at that facility. So the terminal checks the release itself, against a trust root it holds independently of
the node, and the node's check is a courtesy that keeps junk and forks out of its store.

## Decision

1. **Who decides what** (design D1–D3, D5, D6). The **steward signs** window releases on the official ADR-0055 channel;
   sites do not sign, and a deployment or fork repoints the channel by provisioning configuration (D1). **The buck stops
   with the node**: the node decides which release its terminals are offered and when; the steward's signature is what
   makes a release genuine; the terminal re-checks that signature itself and never takes the node's word for it (D2).
   **Terminals are Linux only**: a machine with a full Windows or macOS runs a node instead (D3). **A terminal follows
   every window release**, a same-schema fix to the window included, so a window artifact names its release version
   (D5). **One platform**: `aarch64` on a Raspberry Pi running a Debian-based long-term-support distribution; the build
   targets Debian trixie, the base of Raspberry Pi OS Lite 64-bit (D6). The approach (D4, design §3 A): the bytes travel
   through the existing pinned gateway as database doors; a small root-owned installer on the terminal verifies them and
   swaps the program in; ADR-0111's restart picks it up.

2. **Two new signing contexts, detached signatures, in `cairn-event`.** `CTX_TRUST_ROOT`
   (`application/cairn-trust-root+cbor`) and `CTX_RELEASE_MANIFEST` (`application/cairn-release-manifest+cbor`) join
   ADR-0040's registry. ADR-0055's signatures are detached (several signers, one payload, a content address over the
   payload alone), and `cairn-event`'s existing signer and verifier require an attached payload, so `release_sig.rs`
   adds a public pair per document: `sign_trust_root` / `verify_trust_root_sig` and `sign_release_manifest` /
   `verify_release_manifest_sig`, plus `detached_key_id`, which reads the claimed key (exactly 32 bytes, else
   `BadKeyId`) and proves nothing. They use the same protected header and `external_aad` as every other signature in the
   crate, and the same fail-closed header check, factored out rather than copied. **No public function takes a context
   parameter**: each binds its own, so no caller can pass the wrong one, and a root signature never verifies as a
   manifest, nor either as an event. A blob that carries its own payload is refused as "not detached" (coset's detached
   verifier would panic on one). One COSE implementation stays in the tree, in this safety-critical crate.

3. **The documents** (`cairn-release`'s `docs.rs`; ADR-0055 decisions 2 and 6's shapes). `TrustRoot` (`channel_id`,
   `version`, `root_signers`, `root_threshold`, `release_signers`, `release_threshold`, `prev_root_hash`), each `Signer`
   a `kid` label, a 32-byte Ed25519 `pubkey` and a `custody_note`; `SignedRoot { payload, signatures }`, whose `hash()`
   is the SHA-256 of the payload alone; `Manifest { channel_id, release_version, root_version_ref, artifacts }`;
   `ReleaseMeta { root_chain, manifest, release_signatures, inclusion_proof }`; and `BundleFile { meta, artifacts }`,
   the one file the steward ships, the artifact bytes in manifest order. **One bundle per release, one channel, one
   monotonic `release_version`** (plan-time call). An `Artifact` is typed by `kind` (`window` only today; later
   `extension`, `ddl`, `rebuild-recipe`) and carries its `digest`, `byte_len` and, for a program, `os` (`linux`), `arch`
   (`aarch64`), `platform` (`<ID>-<VERSION_ID>` from `/etc/os-release`, because the window links the system's WebKitGTK
   and glibc) and `generation_line`, the exact line its `--print-schema-generation` prints. The release version is the
   manifest's. So the node and the terminal can choose an artifact without running it.

   The encoding is canonical CBOR, written by the same `ciborium` writer `cairn-event` uses; no field is a `HashMap`,
   and each struct encodes as a CBOR map keyed by field name in declaration order. **Every byte field, the 32-byte ones
   included, is a CBOR byte string** (ruling R4), and `decode` is strict: bad CBOR, a wrong shape, a 32-byte field of
   another length, or any byte after the value is `Malformed`, so a signed document has one reading. The format freezes
   at genesis; this was the cheap time to fix it.

4. **The verifier is pure** (`verify.rs`; ADR-0055 decisions 2, 5 and 6). No I/O, no clock, no database. The node's
   intake and the terminal's installer both call it with what they hold and act only on `Ok`.
   - `genesis` accepts version 1 with no parent, self-signed by at least its own root threshold.
     `advance(history, chain)` climbs one version at a time: version N+1 needs `prev_root_hash` equal to N's hash and at
     least N's `root_threshold` valid signatures from N's root signers; exactly one distinct verifiable payload per
     version is accepted (a repeat of it is harmless). Two different verifiable payloads for one new version, or an
     entry at a version already held whose payload differs and verifies against the version below it, is **`Fork`**,
     never a pick. A differing entry that does not verify is ignored, and so is an unverifiable candidate beside a real
     one; a next version offered with no candidate signed by enough of the version below is **`RootSignaturesShort`**.
     An offered version above the next one with nothing at the next is **`ChainGap`**, which fails closed even for junk,
     and names the version the caller still holds (ruling R10). Another `channel_id` is **`WrongChannel`**. `advance` is
     all or nothing.
   - `verify_release` takes the **newest** verified root (a `VerifiedRoot`, so no caller can hand it signers it chose):
     the manifest decodes, is on that channel, carries at least its `release_threshold` valid signatures from that
     root's release signers, and has a `release_version` above the caller's last one. `root_version_ref` is diagnostic
     only and never chooses the signer set, so a rotated-out release key is dead once the rotation is seen.
     `check_artifact` requires exact length and SHA-256. `verify_bundle` is `advance` then `verify_release`, all or
     nothing: a rotation proved by a bundle whose manifest fails is not kept (ruling R12).
   - **A signature counts by its key** (ruling R6): its `detached_key_id` must equal a trusted signer's `pubkey`, and
     the signature must verify with that key. Each distinct key counts once, however often it signs; anything that does
     not parse counts for nothing. `Signer.kid` stays a label.
   - Before any signature is counted, a root's **shape** is checked, linear in the list length because it runs on
     unverified input: each threshold is at least 1 and no more than its signer count, no list repeats a public key, and
     every key decodes and is **not weak** (small order), else `Malformed` (rulings R8, R11).
   - `VerifiedRoot`'s fields are private, so the only way to hold one is to have verified it (ruling R7). Whoever stores
     roots stores the `SignedRoot`s and **re-verifies them on load** (genesis of the first, then `advance`).
   - **Refusals speak plainly** (`refusal.rs`, one home for all copy, ruling R1). `Refusal::sentence(Who)` says what is
     pinned, what is needed and what to do (naming the node or the terminal where the reader needs to know which is
     refusing), and never says "signature verification failed". The sentences are the plan's, verbatim, plus
     `TerminalFault` (decision 10). **A channel name from an unverified document is sanitised** before it is quoted
     (ruling R9): control characters and the bidirectional overrides and isolates are dropped, and it is cut to 64
     characters, marked "…".
   - `official_root()` returns `None`: **there is no default pin until the genesis ceremony** (#1182), so every intake
     and install without a configured channel refuses with `NoChannel`.

5. **The steward's tool runs offline** (`cairn-release`'s `steward.rs` and binary, behind the default `cli` feature;
   every consumer depends with `default-features = false`, ruling R13). `keygen` writes a `cairn-keystore` CAIRNK1
   sealed key (passphrase from the environment variable `--passphrase-env` names, default `CAIRN_RELEASE_PASSPHRASE`; it
   prints the public key and, once, the recovery code; it refuses to overwrite a file), `genesis`, `rotate` (version and
   parent set by the tool, signed by the PREVIOUS root's signers) and `bundle` (one `window` artifact, `linux`/`aarch64`
   fixed in the tool, its platform and generation line from CI's `window.json`, whose `release_version` must equal
   `--release-version`). `genesis`, `rotate` and `bundle` each verify what they built before writing it, atomically and
   mode 0644. The builders fill digest and length from the bytes, so a signed list cannot disagree with the files it
   ships. **No key lives in CI or on a node**: the tool never runs on either.

6. **The node's store and its three doors** (`db/086`, `SCHEMA_GENERATION` **86**, floor 85: "adds the release store and
   its read-only doors; no window renders a new state"). Four owner-only tables, never synced and never on the clinical
   plane (§7.6; ADR-0013's blob tier is not reused for code): `release_root` (one row per version, kept forever),
   `release_bundle`, `release_artifact` **keyed by `(release_version, idx)`**, the manifest entry, with a plain index on
   `digest`, because identical bytes may be two entries with two platforms (ruling R15), and `release_incident`.
   `cairn_window` holds no grant on the tables, only three read-only `SECURITY DEFINER` doors:
   - `cairn_release_offer(os, arch, platform)`: the **window-kind** artifacts for that target, newest release first,
     with the manifest bytes and signatures, and **no rows while any incident is unresolved**. It judges nothing: **the
     fit is judged in Rust**, by the window (decisions 8 and 9), so the floor rule is never copied into SQL.
   - `cairn_release_roots(after)`: every stored root above `after`, ascending (a NULL reads as 0), so a terminal pinned
     two rotations back can still climb.
   - `cairn_release_chunk(digest, offset, len)`: one slice of a **window-kind** artifact, **sliced in SQL**, at most
     262144 bytes; a negative or missing length or offset is an error, an offset at or past the end is empty, an unknown
     digest or another kind is an error, and **an open incident is an error** too, so the freeze also stops a download
     in progress (ruling R14). The content column is `STORAGE EXTERNAL`, so a slice is read from TOAST without reading
     the whole artifact. The three enter `db/window_door_surface.txt` like any door.

7. **Intake** (`cairn-node release-intake <bundle> [--genesis-root <file>]` and `release-status`, owner verbs on a local
   connection; `release_intake.rs`). Inside **one transaction** it first takes a transaction-scoped **advisory lock**
   (ruling R16), so two intakes serialise and a rival root is seen as a fork, never a unique violation; then it reads
   the stored roots and **re-verifies them** (a stored chain that no longer verifies is refused, not trusted); the
   history is the stored roots, else the supplied genesis, else `official_root()`, else `NoChannel`. Once a root is
   held, a supplied genesis whose payload is the first held root's is accepted (a re-run of the first intake's command
   changes nothing) and a different one is refused (ruling R30). "Newer" means above the newest release the node holds.
   It **verifies everything before its first write**: `verify_bundle`, then every file against the manifest, the bundle
   holding exactly as many files as the manifest lists. Only then do the new roots, the bundle and its files land,
   together or not at all. A **fork** is the one refusal that writes: after the rollback, in its own committed
   statement, it records a `release_incident` (even for a fork inside the first bundle a node takes in), which silences
   the offer door and the chunk door. Each intake then **prunes by key, never by digest**: it keeps the newest two
   releases per `(os, arch, platform)` of kind `window`, deleting those rows by `(release_version, idx)`, and then the
   bundles whose files that prune removed (a digest-keyed delete would take another release's identical row).
   `release-status` says, in plain words, the channel and trust version, the newest release held, and every open
   incident, or that the stored trust file is damaged: the stored roots are re-verified as intake does (ruling R28), so
   one that decodes but no longer verifies is reported as damaged, never shown as the channel. The node's own program
   and schema are still installed the existing way: C2 makes no node upgrade itself.

8. **The window knows its release, and the install check compares it** (design §4.4, D5). A new flag,
   `--print-release-version`, prints one decimal `u64` and exits, read from the raw arguments **before clap**, beside
   ADR-0111's `--print-schema-generation`, whose line is untouched. The build stamps it from `CAIRN_RELEASE_VERSION`;
   absent or unparsable is 0, and a program that cannot answer the flag reads as release 0. `window_floor::install_fit`
   now takes `InstalledBuild { generation, hash, release }`: `Same` when nothing is installed, the build is identical,
   or the **installed release is lower than the running one** (never restart backwards); otherwise ADR-0111's verdict on
   the installed generation, so a same-schema program with a higher release `Fits` and C1's restart takes it.

9. **The window is the courier and verifies nothing** (`release_fetch.rs`; plan-time calls). On ADR-0111's 60 s poll
   (`update_poll`, not a new Tauri command) it may fetch only with a `--release-dir`, outside fixture mode, with the
   installer's `state/status.json` present and whole (a missing file means no installer here) and a platform from
   `/etc/os-release`, and only on a stamp read just then. `due` considers only each release's first program for this
   terminal (lowest `idx`, the one the installer takes, ruling R30; a release whose first program does not qualify is
   skipped whole) and picks the newest that is newer than both the installed and the running release, is at most **64
   MiB** (`MAX_ARTIFACT`), is neither the installer's `refused_release` nor given up this session, and whose program
   **fits** the node's stamp by `install_fit`; an offer whose generation line cannot be read is skipped. So the window
   never fetches a program its node's database would refuse. **An untaken release waits** (rulings R17, R30): while
   anything is at `incoming/complete`, nothing is fetched, even when it is stale or no number: it re-triggers the path
   unit, whose run clears it. **Every fetch counts** (ruling R19): at most **three fetches of one release per session**,
   whatever their outcome.

   The fetch runs at most one at a time, spawned off the poll. It writes `incoming/meta.cbor` (a `ReleaseMeta` with the
   roots after the terminal's pin, the offer's manifest and signatures, and no inclusion proof, ruling R2), then
   `incoming/artifact.bin` in 262144-byte chunks, **each under its own lock of the window's one connection**, so a
   clinical press waits behind at most one chunk (plan-time call: no second connection), then `complete`, holding the
   release number, **written last**, by rename. A stale `complete` is removed first; a slice that is empty or longer
   than asked fails the stage. The file names and the 64 MiB limit live once, in `cairn_release::staging` (ruling R20).
   The update line shows "This terminal is fetching its update from the node." while it runs, "This terminal did not
   install the update: <sentence>" after a refusal, and turns `current` into `waiting` when the status names an
   installed release newer than the running program, a hint only: the restart still asks the installed program first
   (ADR-0111).

10. **The installer re-verifies against its own pin** (`cairn-terminal-install`, root, started by a systemd path unit on
    `incoming/complete`; Unix only). It loads **its own** pin: `state/roots.cbor` re-verified; else the trust file
    whoever set the terminal up installed (`/etc/cairn/channel-root.cbor`); else the compiled-in root (none until the
    ceremony); else no pin, which refuses `NoChannel`. A damaged `roots.cbor` is the terminal's own fault, never a fall
    back to genesis, which would undo an accepted rotation. Nothing the node or the window says is trusted.
    - With no `complete`, the window may still be writing: the installer touches nothing, and writes an idle status only
      if none exists; but a pin it cannot load (a wrong trust file) writes the `TerminalFault` status over any it finds
      and fails the run, so `install.sh` stops (ruling R29).
    - **A fork incident first**: an incident file (unreadable, or its presence uncheckable, reads as version 0, still
      frozen) refuses `Fork` before a staged byte is read, until a human removes it. Only the terminal's own faults (a
      damaged pin or `installed_release`, a bad `--run-as`) are said before it, as `TerminalFault`. Then every staged
      file is read as hostile: `O_NOFOLLOW` (a link is refused), `O_NONBLOCK` (a FIFO cannot hang it), regular files
      only, each read capped. `complete` must be a bare decimal and equal the manifest's release, else `Malformed`.
      `decide` then refuses a file over 64 MiB (`TooLarge`) before reading it, runs `verify_bundle` against this
      terminal's pin and last installed release, and picks the `window` artifact whose `os`, `arch` and `platform` all
      equal this machine's, else `NoProgramForThisTerminal`; the bytes read into memory must pass `check_artifact`.
    - **The start check runs the verified bytes, never as root** (ruling R21). The installer writes those in-memory
      bytes to `<program>.new` (atomic, mode 0755), and runs THAT file, never the staged one, which its unprivileged
      owner could change after the check, as the `--run-as` user (default `cairn-window`; a missing user, or one with
      uid 0 or primary gid 0, is refused as the terminal's own fault), with an empty environment, `/` as its directory
      and a 5 s timeout per flag. It must print the artifact's generation line and the manifest's release, one line
      each, else `.new` is removed and the release is `WontStart`. This proves the program loads its libraries here
      before it replaces anything, so no revert is needed.
    - **The swap is atomic and keeps `.prev`**: the current program is hard-linked to `<program>.prev` (an older one
      replaced; none on a first install), `.new` is renamed over the program, and the directory is fsynced.
    - **Recording**: on success the new `roots.cbor` and `installed_release` first; a fresh fork writes
      `state/incident`; then **`status.json` is always written first, and `incoming/` is then cleared on every outcome**
      (rulings R17, R19), `complete` first, so the path unit cannot loop and a polling window never sees an empty folder
      beside a stale status. `status.json` (mode 0644, `InstallerStatus`, one definition shared with the window, ruling
      R3) holds `state`, `installed_release`, `pinned_root_version`, the refusal's `sentence` and `refused_release`.
    - **`refused_release` is set only for refusals reached after the release's signatures verified that a re-fetch
      cannot fix: `NotNewer` and `NoProgramForThisTerminal`** (ruling R23). Every other refusal leaves it empty, because
      its release number may come from a forged meta that would otherwise lock out the genuine release; `WontStart` too,
      because a slow first start on a Pi can time out. The window's three fetches per session bound the re-fetching.
    - **The installer's own faults say so** (ruling R22). Exactly these become `TerminalFault`: a damaged or unreadable
      `roots.cbor` or `channel-root.cbor` (the pin cannot be loaded, staged or not), a damaged `installed_release`, a
      `--run-as` that names no user, root or a user in group 0, and a failed write while putting the program in place
      (`.new`, `.prev`, the rename, the directory fsync). They say "This terminal could not install the update because
      of a fault on the terminal itself; the program in place keeps running. Tell whoever looks after this terminal.",
      never `WontStart`'s sentence (no precise untruth), with `refused_release` empty, and still write the status and
      clear `incoming/`. A failed write of `roots.cbor` or `installed_release` AFTER a successful swap is only logged:
      the status still says `installed`, the new program is in place, and the recorded release may be the old one
      ([#1184](https://github.com/cairn-ehr/cairn-ehr/issues/1184)).

11. **Packaging and the release build** (`packaging/terminal/`, `.github/workflows/release-build.yml`). **A terminal is
    remote by definition** (ruling R26): it reaches its node through the pinned gateway, and a machine with the database
    runs a node. So `install.sh`, the first hand install and the terminal's genesis, requires `--conn` (or
    `CAIRN_WINDOW_CONN`) and `--node-fingerprint` on a first install; a re-run keeps `/etc/cairn/window.env` unless both
    are given again, and never replaces an existing `/opt/cairn/bin/cairn-window` (the installer owns updates) without
    `--replace-window`. It installs `cage` (MIT) from apt as the kiosk compositor, creates the `cairn-window` user, sets
    `incoming/` to `root:cairn-window` 2770 and `state/` to root 0755, never silently replaces an existing different
    trust file, and prints the trust file's SHA-256 to compare with a second channel. **It records the hand-installed
    program's release** (ruling R29): when `state/installed_release` is absent or `--replace-window` was given, it
    writes it (root, 0644, the installer's bare decimal) from `cairn-window --print-release-version`, asked as
    `cairn-window` (a dev build answers 0), so the installer's "newer" gate never starts below the program in place and
    a compromised window cannot have root downgrade it to an older signed release. After enabling the path unit, and
    before enabling the kiosk, **it starts the installer once** (`systemctl start cairn-terminal-install.service`, so it
    never runs beside a run the path unit started; ruling R27): the window fetches nothing until `state/status.json`
    exists, and the path unit starts the installer only once a release is staged, so with nothing staged this run writes
    the idle status carrying the terminal's trust version (0 with no trust file at all). The script removes any old
    status first (forgetting at most a release refused for good, which the window may fetch once more) and stops, naming
    the trust file, unless that run wrote `"state": "idle"` (ruling R29). **The kiosk never runs as root**
    (`User=cairn-window`, `cage -- cairn-window --terminal-dir … --release-dir /var/lib/cairn-terminal`); the installer
    service runs as root, `KillMode=control-group`, with `ProtectSystem=strict` and write access to `/opt/cairn/bin` and
    `/var/lib/cairn-terminal` only; the path unit's trigger is limited to 10 runs a minute. `fs.protected_hardlinks=1`
    is a stated requirement. CI builds the **unsigned** `aarch64` window and installer on an `ubuntu-24.04-arm` runner
    in a `debian:trixie` container (named by tag, not digest; the rustup bootstrap is unpinned), writes `window.json` by
    running the built window's two print flags, and uploads the files with their SHA-256 sums; it holds no secret and
    has no signing step. **The steward signs offline**: until reproducible builds exist (#259), the steward signs what
    CI built.

**Paper-parity.** Not clinical-surface: the slice moves signed program files from the node to its terminals, and no
clinical workflow gains or loses a step (the restart it leads to is ADR-0111's). RUNBOOK §41 owes the figures, by a
human on a node and a Pi: intake time for one bundle, LAN download time, the longest press delay during a download
(budget ~200 ms), swap time, and intake to the calm line on the terminal.

## Consequences

- The owner takes in one bundle per release, and each terminal fetches, re-verifies and installs it with no human act at
  the terminal, then restarts through ADR-0111's path. A same-schema fix to the window reaches every terminal.
- A broken-into node, or a broken-into window, cannot put on a terminal any program the steward did not sign: the
  installer verifies against its own pin, test-runs the verified bytes unprivileged and installs those bytes.
- The node holds one window artifact per release per platform (the design's estimate: ~20 MB), two releases deep, in its
  database.
- Each terminal carries a second program and three systemd units, and its first install is still by hand.
- ADR-0055's verifier core exists, with its gate order and refusals, for the window. The node's own load gate for DDL
  and extensions can add its artifact kinds to the same bundle without a new format.

## What is still not guaranteed

- **No default pin until the genesis ceremony** ([#1182](https://github.com/cairn-ehr/cairn-ehr/issues/1182)).
  `official_root()` is `None`; a node needs `--genesis-root` and a terminal a trust file in `/etc/cairn/`, both given by
  hand. The ceremony (offline root and release keys, ADR-0026's escrow rungs, root version 1 published with its
  fingerprint on a second channel, 1-of-1 as ADR-0055 decision 8 declares) is a human act owed by the maintainer; the
  root file lands in `crates/cairn-release/official/` in its own PR after it.
- **A node restored to an older release** may leave a terminal with a newer window than its database; the installer
  never goes back, so that terminal needs a hand fix ([#1183](https://github.com/cairn-ehr/cairn-ehr/issues/1183)).
- **A crash between the swap and the record** leaves the new program in place with `installed_release` still old; a
  signed release in between could then be installed over it
  ([#1184](https://github.com/cairn-ehr/cairn-ehr/issues/1184), ruling R24; it needs an intent-file design).
- **Unknown artifact kinds are never pruned**: intake stores and digest-checks them and the doors never serve them, but
  pruning counts only `window` files, so a bundle holding another kind is kept forever
  ([#1185](https://github.com/cairn-ehr/cairn-ehr/issues/1185)).
- **The inclusion proof is carried, not verified** ([#258](https://github.com/cairn-ehr/cairn-ehr/issues/258)). ADR-0055
  decision 7 says the official channel MUST log every release; the bundle has the slot and intake stores it, but the
  window stages none and nothing checks one. Until the log exists, a release key used for one targeted facility is not
  detectable as evidence, and `release-status` reports no log freshness.
- **No reproducible builds** ([#259](https://github.com/cairn-ehr/cairn-ehr/issues/259)): the steward signs what CI
  built, so a compromised CI that builds a different program than its source gets it signed unless the steward catches
  it. CI's `debian:trixie` image is a tag, not a digest, and its rustup bootstrap is unpinned, so what CI built also
  depends on what those fetched that day.
- **No co-signer floors** (ADR-0055 decision 6, [#1187](https://github.com/cairn-ehr/cairn-ehr/issues/1187)), **no
  freshness rung** (§7.9, [#260](https://github.com/cairn-ehr/cairn-ehr/issues/260)), and ADR-0055's revocation latency
  holds: a terminal that has not yet seen a rotation still honours the release key it rotated out. A 1-of-1 steward root
  makes the steward's offline machine the single point of compromise.
- **The node's own load gate is not built** ([#257](https://github.com/cairn-ehr/cairn-ehr/issues/257)): a node's
  program, DDL and extensions are still installed the existing way, unverified by this plane.
- **Only an honest window follows the node's choice.** The installer checks signature, newness and platform, not fit: a
  compromised window could stage any signed release newer than the installed one, even one the node does not serve, and
  a program the node's database refuses would then not open at its next launch. It still cannot stage an unsigned one,
  nor an older one: on a fresh terminal "installed" is the hand-installed program's own release, which `install.sh`
  records (ruling R29); a terminal set up with a dev build records 0, so any signed release is newer there.
  No staging quota is set ([#1186](https://github.com/cairn-ehr/cairn-ehr/issues/1186)): the 64 MiB limit bounds what
  the installer reads, not what the window's user can write to the folder's filesystem.
- **No verb resolves an incident.** On the node it stays open until someone sets `release_incident.resolved_at` by hand;
  on a terminal, until root removes `state/incident`. ADR-0055 decision 5's audited resolution ceremony is not built.
- **The privilege drop is untested, and the units have never run on a Pi.** The tests run the start check as the test's
  own user (dropping privilege needs root). The units and `install.sh` have run on no real Pi; RUNBOOK §41 is owed by a
  human.
- **The window fetches only on its 60 s poll and on a fresh stamp**, so a terminal learns of a release up to a minute
  late, and three fetches of one release cut off by node outages in one session give it up until the window restarts
  ([#1190](https://github.com/cairn-ehr/cairn-ehr/issues/1190)).
- **After a release-signer rotation the node still offers a kept release no terminal accepts**
  ([#1188](https://github.com/cairn-ehr/cairn-ehr/issues/1188)): the installer checks a manifest against the newest
  root's release signers only, so the older of the two kept releases is refused as "not signed by enough of the Cairn
  project's release signers", up to three times per window session.
- **Refusals that are not the release's fault still cost three downloads per session**
  ([#1189](https://github.com/cairn-ehr/cairn-ehr/issues/1189)): `Fork`, `NoChannel` and `TerminalFault` leave
  `refused_release` empty (ruling R23), so the window cannot tell them from transit damage.

Pins: `cairn-event`'s `release_sig.rs` tests; `cairn-release`'s `docs_tests`, `verify_tests`, `verify_release_tests`,
`refusal_tests`, `staging_tests`, `status_tests`, `steward_tests` and `tests/cli.rs`;
`crates/cairn-node/tests/{release_store_doors,release_intake,window_door_surface}.rs`,
`src/{release_intake_tests,window_floor_tests}.rs`; `cairn-gui-tauri`'s `update_tests.rs`, `release_fetch_tests.rs`,
`release_fetch_db_tests.rs` (DB-gated; CI does not run them, #905) and `update_webview_tests.rs`;
`cairn-terminal-install`'s `decide_tests.rs`, `run_tests.rs`, `files.rs` and `swap.rs` tests and `tests/packaging.rs`.
RUNBOOK §41 carries the human measurements and the on-screen refusals.
