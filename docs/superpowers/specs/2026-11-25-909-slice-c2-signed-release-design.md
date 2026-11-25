# #909 slice C2: a terminal follows its node — the signed window release — design

Status: **approved 2026-11-25**, brainstormed with the maintainer. Parent:
[`2026-11-24-909-slice-c1-rollout-design.md`](2026-11-24-909-slice-c1-rollout-design.md) §9 (and its D4). Builds the
first code of [ADR-0055](../../spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md)
(issue #257's verifier) on top of [ADR-0110](../../spec/decisions/0110-the-pinned-terminal-gateway.md)'s pinned link and
[ADR-0111](../../spec/decisions/0111-an-older-window-works-above-a-floor.md)'s restart path.

## 1. Why

After C1, an upgraded node no longer disturbs its terminals, but each terminal still needs someone to install the new
window program on it by hand. A facility with ten terminals and no IT walks to ten machines; until it does, a terminal
below the floor finishes its chart and then does nothing useful (C1 design §6). The maintainer's D4 (2026-11-24): the
owner upgrades the node **once**, and every terminal follows by itself.

The danger in "follows by itself" is plain: a program that arrives over the network and replaces the window is the
program every clinician types their passphrase into. If a terminal accepted whatever its node served, one broken-into
node would own every clinician's signing key at that facility. So the terminal checks the release itself, against a
root it holds independently of the node.

## 2. What the maintainer decided (2026-11-25)

- **D1. The steward signs window releases** (the official ADR-0055 channel; ADR-0055 decision 1's default anchor). Sites
  do not sign. A national deployment or fork repoints the channel by provisioning configuration (ADR-0055), unchanged.
- **D2. The buck stops with the node.** A node is the autonomous unit that syncs with other nodes; a terminal is an
  auxiliary of its node, not a peer. The node decides which release its terminals get and when; the steward's
  signature is what makes a release genuine; the terminal re-checks that signature itself (§1) and never trusts the
  node's word for it. (Played back and confirmed: "nothing wrong".)
- **D3. Terminals are Linux only.** A machine running a full Windows or macOS should run a node instead (sign-in
  slice 5), not be a terminal. No macOS notarisation, no Windows code-signing in this slice.
- **D5. A terminal follows every window release**, including one that changes only the window with the same schema
  (a fix to the screen clinicians type their passphrase into must reach every terminal). So a window artifact carries
  its release version, and C1's install check compares it as well as the generation line (§4.1, §4.4).
- **D6. One terminal platform: ARM (`aarch64`) on a Raspberry Pi, a minimal Debian-based distribution with long-term
  support** — terminals are the minimalist answer for ultra-low-resource sites. One platform means one release build
  per release. Which Debian release that is (the one current Raspberry Pi OS Lite 64-bit is based on, at build time)
  is the builder's to check; the manifest's `platform` names it, so a later platform is additive.
- **D4. Approach A** (§3): the bytes come through the existing pinned gateway as database doors; a small root-owned
  installer on the terminal verifies and swaps the program in; C1's restart picks it up.

## 3. Approaches considered

All three share the verifier (§4.1), the node's intake (§4.3) and the steward's signing (§4.2). They differ in how the
bytes reach a terminal and who puts them in place.

- **A. Through the gateway; a root-owned installer (chosen).** The node keeps verified releases in a local table that
  never syncs; the window reads its architecture's artifact in chunks through `cairn_window` doors over the link it
  already has and drops it into a staging folder; a root-owned systemd unit verifies it again and swaps it in. The
  window is only the courier: a broken node or a broken window cannot install anything the steward did not sign. Costs:
  a second small program on the terminal; ~20 MB per release per architecture in the node's database.
- **B. Like A, but over a second pinned TLS port** that the terminal service serves as files. Keeps binaries out of
  Postgres, but adds a second listener to pin, admit and revoke — twice slice B's surface — for a saving that pruning
  old releases already gives. Rejected.
- **C. The window verifies and replaces its own file** in a folder its user can write. Smallest, but anything running
  as that user, a compromised window included, can install anything; the signature check becomes decoration. Rejected.
- *A `.deb` repository served by the node* (dropped before the comparison): apt brings its own GPG trust, giving two
  trust models for one program.

## 4. The design

### 4.1 The verifier (issue #257's pure core)

A new root-workspace library crate (name left to the builder; `cairn-release` is a candidate) that is **pure: no
database, no network, no clock, no filesystem**. It takes bytes and a pin and returns a verdict. It implements, for the
whole release bundle (ADR-0055 decision 6's shape):

1. The chained trust-root document (ADR-0055 decision 2): content address over the payload alone; signatures detached;
   version N+1 needs ≥ `root_threshold` valid signatures from version N's `root_signers`; the pin advances only to the
   highest verified version; regression refused.
2. **Fork detection** (decision 5): two distinct verifiable payloads for one version → a `Fork` verdict, never a pick.
   The caller freezes (§4.3, §4.5).
3. Release signatures: ≥ `release_threshold` valid signatures from the release set of the **newest verified** root
   version; `root_version_ref` is diagnostic only.
4. Every artifact digest checked against the manifest before an artifact is reported usable; release versions
   monotonic per channel against the caller's last installed one.
5. Verdicts carry what a refusal needs in practice-manager words (pinned version, needed version, what is missing,
   what to do next; ADR-0055 decision 6, trust-root design §3.5) — never "signature verification failed".

Signatures are COSE_Sign1 + Ed25519 with **new, distinct signing contexts** for a root version and a release
manifest, added to `cairn-event`'s closed, add-only context registry, so no event signature can verify as either.
ADR-0055's signatures are **detached** (the content address covers the payload alone), and `cairn-event`'s signer and
verifier are private and require an attached payload (`cose_sign1_in_context`, `cose_verify1_parsed`), so C2 adds a
small public detached sign/verify pair to `cairn-event`: a change to a safety-critical crate, reviewed as one (my
call: one COSE implementation in the tree, not a second in the verifier crate). Encoding is canonical CBOR, as events
are (ADR-0015).

**One bundle per Cairn release, one channel, one `release_version`** (my call). The bundle's artifact list is typed by
**kind** (`window`, and later `extension`, `ddl`, `rebuild-recipe`), each kind carrying its own fields: a window
artifact has no Postgres-major. ADR-0055 makes the bundle shape a day-one commitment, so the kind tag is what lets the
node's own load gate (#257) add its artifacts later without a new format. The node's intake and the terminal's
installer both read "newer" as a higher `release_version` on that one channel.

**Manifest content for a window artifact** (the part this slice adds to the bundle shape): per artifact, `os` (`linux`
only today), `arch` (`aarch64` only today, D6), `platform` (the distribution and release it was built for, because the window
links the system's WebKitGTK and glibc; D6), the digest, the byte length, and the window's **generation line** — the exact
`<generation> <64 hex>` its `--print-schema-generation` prints (C1's durable rule: that line never changes). The node
and the terminal can then choose an artifact without running it; C1 still runs the installed program before any restart.
The window artifact also names its **release version**, so a release that changes only the window (same schema) is
still a newer program (§4.4, D5).

**Out of the verifier in C2:** the transparency inclusion proof is carried in the format but **not verified** (#258
builds the log); co-signer floors (policy, §7.9); the node's own DDL and extension load (#257's load-gate half). See §6.

### 4.2 The steward's side

- **A small signing tool** (a binary of the verifier crate, or its own; builder's call) with: make a genesis root
  document; publish root version N+1; assemble a bundle from built artifacts; sign a manifest. It never runs on a node
  and is never built into the window.
- **Keys never live in CI or on a node.** CI builds the window for D6's one platform (`aarch64`, Debian) and publishes the
  artifacts and their digests; the steward reviews them and signs the manifest on their own offline machine (the
  "daily pen", ADR-0055 decision 3). A CI that held the release key would make a CI compromise signed code on every
  terminal. Until reproducible builds exist (#259), the steward signs what CI built: stated, not hidden.
- **The genesis ceremony is a human act owed by the maintainer:** generate the root key (offline, escrowed per
  ADR-0026's rungs) and the release key, publish root version 1 with its fingerprint on a second channel. 1-of-1, as
  ADR-0055 decision 8 declares.
- **The official root version 1 is a file in the repository**, compiled into `cairn-node` and the terminal installer as
  the default pin, and overridable by provisioning configuration (a root-owned file) for another channel. **Until the
  genesis ceremony, there is no default pin** (my call): intake and the installer refuse every bundle with a sentence
  that says no release channel is configured; tests mint their keys at runtime (house rule 6). The slice merges
  without waiting for the ceremony; the root file lands in its own small PR after it.
- **CI release builds are new work.** Today CI runs only x86_64 test jobs on ubuntu-24.04 and builds no release
  artifact; the aarch64 Debian build (native ARM runner, or a Debian arm64 container) and the artifact publishing are part of this slice's cost.

### 4.3 The node: intake, store, serve

- **Intake** is one owner verb (name left to the builder; e.g. `cairn-node release-intake <bundle>`): verify the bundle
  with §4.1 against the node's pin, refuse in practice-manager words or store it. Sneakernet is first-class: the bundle
  is one file. The node's own program and schema are still installed the existing way; C2 does not make a node upgrade
  itself.
- **The node's pin** (channel, highest verified root version, and any frozen-fork incident) is node state; a fork
  freezes intake for that channel and says so, and changes nothing that runs (ADR-0055 decision 5).
- **The store** is a local table that **never syncs** and is never on the clinical plane (§7.6: the sync plane never
  carries code; ADR-0013's clinical blob tier is not reused for the same reason). It holds the verified bundle's
  manifest, signatures and chain segment, and the window artifacts' bytes. Old releases are pruned; the builder picks
  the rule (e.g. keep the newest two per architecture).
- **The node keeps every root version it has verified** (small and rare), not only one bundle's chain segment.
- **The doors**, granted to `cairn_window`, are new and additive (they enter `db/window_door_surface.txt` like any door,
  so their signatures must be right first time): (1) "the releases my node holds for `(os, arch, platform)`, given that
  I am pinned at root version *v*" — returns each stored window artifact's release version and generation line, plus
  the manifest, signatures and the root versions after *v*; (2) "chunk *k* of artifact *d*". Read-only. **The fit is
  judged in Rust, not SQL** (my call): the window applies C1's `install_fit` to the generation lines the door returns,
  so the floor rule is never copied into SQL. The node does not decide on the terminal's behalf whether to install: the terminal's
  installer does (§4.5).
- **Which release, and when, is the node's** (D2): the offer follows what the owner has taken in and the node's own
  stamp. A terminal never gets a window newer than its node's database.

### 4.4 The window: the courier

- On C1's 60 s poll, the window asks the door (§4.3, 1) whether its node holds a release for this machine's
  `(os, arch, platform)` **newer than the installed program's release version** whose generation line fits the
  stamp. This is not gated on C1's "update waiting": a window-only release with the same schema must also be fetched
  (D5). C1's `install_fit` and its `InstallView` then compare the release version as well as the generation line, so
  the restart path also restarts into a same-schema newer program. The installed program reports its release version
  through a **new** flag beside `--print-schema-generation`, never by changing that flag's line (C1's durable rule: an
  older window parses it); a window built before C2 cannot see a same-schema release, so it follows schema changes
  only until its first C2 install.
- If one is offered and not already staged, it fetches the artifact chunk by chunk into the staging folder (§4.5),
  together with the manifest, signatures and chain segment, and writes a completion marker last.
- **No clinical press may wait behind the download.** The window has one shared database connection (#807's design);
  a chunk must hold it no longer than a short bound, with the press always first, or the download uses a second
  connection opened only while fetching. Which, and the chunk size, are the builder's; the bound is not: a press
  never waits more than ~200 ms behind a chunk, and the 15 s cut-off (#875) is never reached because of a download.
- The window **verifies nothing and trusts nothing it fetched**. It may show the installer's state (§4.5) on C1's calm
  line ("Update downloading", "Update ready", or the installer's refusal sentence); it never claims an install it did
  not see.
- After the installer swaps the program in, C1's existing path does the rest unchanged: `installed_program` reads the
  new program's generation line, and the idle re-lock or Update now restarts into it.

### 4.5 The terminal installer

- A small root-owned Rust program (a binary of the root workspace, name left to the builder), started by a systemd
  **path unit** that watches the staging folder's completion marker. Safety-critical (§9.1): small, TDD, legible.
- **Layout** (builder may rename, not restructure): the program at `/opt/cairn/…/cairn-window` (root-owned, mode 755);
  the staging folder writable by the window's user group only; the pin, the installed release version and any fork
  incident in a root-owned state directory; the default pin compiled in (§4.2) and an override in `/etc`.
- On a marker it: (1) verifies the staged bundle with §4.1 against **its own** pin — never against anything the window
  or node says; (2) checks the artifact's `os`/`arch`/`platform` match this machine and its release version is newer than the
  installed one; (2a) **runs the new program with `--print-schema-generation`** (as an unprivileged user, with a
  timeout) and refuses it unless it prints the manifest's generation line: that proves it loads its libraries on this
  machine before it replaces anything (my call; a program that cannot start never reaches the swap, so no revert is
  needed); (3) writes the artifact beside the program, `fsync`s, and **renames it over the program atomically**,
  keeping the previous program as a fallback; (4) records the new version and pin; (5) clears the staging folder.
  Any failure clears the staging folder and leaves the running program and the installed one untouched.
- **The rename keeps C1 working — in code:** C1's `exe_path` reads a replaced running program's `(deleted)` path and
  asks the file now at that path. That has never been run on a real Linux terminal (RUNBOOK §40 is owed); C2 rests on
  it, so the plan runs it early on Linux, not last.
- It writes one short status file the window may read (state + a practice-manager sentence) and logs to the journal.
- **Caps:** it refuses a staged file larger than a fixed ceiling and the staging folder has a quota, so a compromised
  window can at worst fill a bounded folder.
- **A minimal image has no desktop.** A Lite image runs no graphical session, so the first install also brings a
  single-app kiosk compositor (licence checked before it is chosen) that starts the window at boot. The installer's
  pre-swap check (2a) runs without a display, which `--print-schema-generation` already allows (it exits before any
  window opens).
- **The first C2-capable program is installed by hand** (the C1 way), together with the installer and its units, as a
  package or script the builder chooses. That is the terminal's genesis: from then on it follows its node.

### 4.6 The order a release travels

1. CI builds; the steward signs the bundle offline and publishes it.
2. The owner upgrades the node the existing way, then takes the bundle in (§4.3). (Either order works: a release whose
   window does not fit the stamp is stored but not offered until it does.)
3. Each terminal's 60 s poll (C1) sees the new stamp; the window asks for the offer, downloads, marks complete.
4. The installer verifies and swaps; the window's next install check sees `fits`; the idle re-lock or Update now
   restarts it.

## 5. Goals, non-goals, anti-goals

- **Goals:** the owner upgrades once (one intake per release) and every Linux terminal follows with no human act; no
  terminal ever runs a program the steward did not sign, even if its node or its window is compromised; a refusal says
  in plain words what is wrong and what to do; a download never delays a clinical press.
- **Non-goals:** macOS or Windows terminals (D3); a node upgrading its own program, DDL or extension from the bundle
  (#257's load gate); the transparency log (#258) and reproducible builds (#259); co-signer floors and freshness rungs
  (§7.9, #260); rolling a terminal back to an older release.
- **Anti-goals:** the terminal trusting its node's word for a release; a release key in CI or on a node; a confirmation
  dialog anywhere in the path; interrupting a clinician to install (C1's rules govern the restart).

## 6. Risks and stated limits

- **No transparency log yet.** ADR-0055 decision 7 says the official channel MUST log every release; C2 ships the
  slot, not the check (#258). Until it exists, a release key used by someone else for one targeted facility is not
  detectable as evidence. ADR-0055's tripwire (≥ 2-of-3 root before a deployment outside the steward's control) is
  untouched by C2.
- **A 1-of-1 steward root** (ADR-0055 decision 8): the steward's offline machine is the single point of compromise.
- **Revocation latency:** a terminal that has not seen root version N+1 still honours a release key N+1 rotated out
  (ADR-0055 decision 6, stated there).
- **Pi resources.** WebKitGTK on a Pi 4/5 is workable; smaller boards are not a target. The download and the
  pre-swap check run beside a clinician's work on a small machine, so the plan's RUNBOOK section measures press delay
  on the Pi itself (§7).
- **A node restored to an older release** may leave a terminal with a newer window than its database; the installer
  refuses to go back, so that terminal needs a hand fix. Rare; recorded in the RUNBOOK.
- **Genesis is procedural:** a poisoned first hand-install carries a poisoned pin (ADR-0055 decision 7). The
  first install's fingerprint check is the defence.

## 7. Paper-parity

Paper-parity: not clinical-surface — the slice moves program files between machines; no clinical workflow gains or
loses a step (the restart it leads to is C1's, measured in RUNBOOK §40). The plan's RUNBOOK section owes: intake time
for one bundle; download time on the LAN for one artifact; the longest press delay measured during a download (budget
~200 ms); installer swap time; the end-to-end time from intake to a terminal showing "Update ready".

## 8. Left to the builder

- Crate and binary names; whether the signing tool is a binary of the verifier crate.
- The exact CBOR field names of the root document and manifest, within ADR-0055's shape and §4.1's window fields.
- The store table's shape, the pruning rule, and whether it rides backups and the local-state export.
- The chunk size and the second-connection-or-yield choice within §4.4's bound.
- The terminal's file layout names, the path unit's details, the staging quota and size ceiling, and the package or
  script for the first hand install.
- The status-file format and the calm-line copy, checked against the house words.
- Whether this needs a new ADR (it does: it decides the window artifact's manifest fields, the terminal's own pin,
  and that the steward signs offline) and how it cites ADR-0055, ADR-0110 and ADR-0111.
- The three-lockfile refresh a new root-workspace crate needs; the window should not need the verifier crate at all.
