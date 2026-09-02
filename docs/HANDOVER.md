# HANDOVER — Cairn

## ⇒ NEXT

> [!WARNING]
> **⇒ THE DISASTER-RECOVERY HOLE IS HALF-CLOSED, AND *WHICH HALF* IS THE WHOLE POINT.
> [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495) IS CLOSED.
> [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) IS NOT.** *"The DR hole is fixed"* is exactly
> the true-in-part sentence that gets quoted as complete six months later — the failure mode this slice
> exists to correct. **Under-claiming is safe here; over-claiming is the defect.**
>
> - **✓ #495 — THE KEY. CLOSED** by
>   [ADR-0066](spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md) (spec v0.68), built
>   2026-08-24. The node's X25519 unwrap secret is no longer HKDF-derived from its Ed25519 signing seed:
>   it is an **independent keypair** sealed in its own `<key>.unwrap` file, riding the `CAIRNL1`
>   local-state export beside surviving `event_dek` rows (a **shredded** event's DEK excluded by
>   construction), and `restore` **adopts** it instead of minting one. **A restored solo node can now
>   inherit its custody KEY** — its own custody *records* are a different question, see #500 below.
> - **✗ #500 — THE BYTES. STILL OPEN.** `backup.rs::read_event_set`
>   still exports `SELECT signed_bytes FROM node_event`: the medium is the **federation plane only**, so
>   NO `event_log` row travels — not clinical, not demographic, not identity, not registration. A solo
>   clinic backs up nightly, `verify-backup` passes, health is reported honestly, and restore recovers who
>   it peered with and **zero patients**. Pinned by
>   `dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event`; **slice 2d
>   inverts it — nothing built so far does.**
> - **○ SLICE 2a LANDED 2026-08-31 AND CLOSES NOTHING.** New crate `crates/cairn-medium` (today's
>   `medium.rs` extracted verbatim, split by responsibility, the #503 pattern) plus **CAIRNB3**: each
>   plane-tagged segment now carries its own signed, chained attestation instead of CAIRNB2's whole-set
>   head marker, so appending costs one signature, not a full re-sign and rewrite. **No database read
>   exists in this crate, by construction** — the medium still carries no clinical event until 2c writes
>   one. **Next build is 2b** (the transport seam + the paged pull). Detail in ROADMAP's slice 2a entry.
> - **⇒ 2a's REVIEW WAVE RESHAPED THE CRATE'S API (2026-09-01), while it had no consumers.** A mutation
>   audit found **19 of 19 single-line mutations surviving**; `wire_pins.rs` pins the wire constants as
>   golden bytes and it re-runs **18/18 killed**. Four false all-clears are closed by **`health::assess`
>   — the one composed verdict, and the entry point callers should use** (`intact()` → `chain_intact()`).
>   **`Plane::Unknown(tag)`** is first-class; `BackupError` splits three ways. **#522 is LOUD**
>   (`IndexMismatch`); **#523/#524 open as filed**; **#525 done**. Detail: the 2026-09-01 entry.
> - **⇒ #527: THE ALERT LIST IS READABLE NOW, AND THE ASSUMPTION IN IT WAS WRONG (2026-09-02).**
>   `scripts/codeql-alerts.sh` reads it (read-only, script-shaped because `gh api` is deny-listed
>   repo-wide and must stay so). **30 open alerts; the critical 18 were a REAL defect, not the
>   #146/#520 false-positive class** — see the 2026-09-02 entry. **Two human acts still owed:**
>   dismiss the 12 `rust/cleartext-logging` alerts (per-alert verdicts are in #527's comment), then
>   **make the `CodeQL` gate a REQUIRED check** — but only in that order: a permanently-red required
>   check trains everyone to merge past it, which is how a genuine critical sat unread for a week.
> - **⇒ THE LOOSE END NAMED SO IT CANNOT BE LOST.** Custody must apply the same `erasure_shred_log`
>   exclusion on **both** the medium path and the `CAIRNL1` export path, or a shredded body comes back on
>   restore. 2a only makes both expressible; **2c decides which is authoritative.**
>
> **Slice 1 therefore hands a restored node a working key and nothing yet to open with it — 2a does not
> change that.** Neither half is useful alone. **Never cite ADR-0026 decision 1's clinical promises as
> met.** Its promise 2 —
> *"node-default data-at-rest keys survive"* — has **no subject at all**: no node-default key tier exists,
> so it is neither honoured nor violated and must not be read as satisfied by anything slice 1 did.
> **[#502](https://github.com/cairn-ehr/cairn-ehr/issues/502) — items 1–3 fixed; item 4** (a discarded
> keystore-load reason) **stays open.** An unreadable export now refuses the restore instead of being
> skipped in silence; a corrupt `.lsk` sidecar is diagnosed present-but-unusable rather than "absent";
> `verify-backup` refuses a zero-event medium instead of `backup OK: 0/0`, and no longer prints an
> all-clear it had not established.
>
> **✓ FEDERATED SYNC WORKS AGAIN — [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503) IS CLOSED**
> (2026-08-30, detail in Recent sessions below). New `crates/cairn-keystore` carries the sealed key-file
> format both binaries need, and `cairn-sync` now **LOADS** the provisioned key at startup, resolved
> **once** instead of derived independently at six sites — **one derived path survives by design**, trap 3.
>
> **The reusable lesson, and the reason this hid for weeks:** *a deferral is only honest while its stated
> precondition holds, and nothing in the repo watches for one expiring.* The `localstate.rs` module header
> declared its seam truthfully — *"the federation-node tier has no clinical surface yet"* — and ADR-0052
> made that false without reopening it, while ROADMAP kept recording slices A–D as ✓ done. **Before
> trusting any ✓, check whether the sentence that justified it is still true.**

> [!IMPORTANT]
> **Five traps slice 1 minted. Each is a step a next session takes in good faith.**
>
> 1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY** — a pre-ADR-0066 node re-derives its old
>    secret exactly once, inside `keystore::adopt_derived_unwrap_secret`, keeping its `event_dek` rows
>    openable. **Calling it anywhere else re-creates the #495 coupling.** Pinned by
>    `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`, which sweeps **every shipping tree**
>    (`sources::PRODUCTION_TREES` — `crates/`, `extensions/`, `cairn-gui/`, both `exclude`d trees
>    included). Its allow-list asserts every entry is still live — a dead entry fails the guard, so it
>    cannot quietly widen. **When it fails, delete the entry; never add one.** The sweep and the
>    test-gate matcher (`is_a_test_gate_attribute`, which must recognise pgrx's
>    `#[cfg(any(test, feature = "pg_test"))]`) move **together** — widening either alone reddens the
>    guard on correct code.
> 2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6).
>    `ensure_unwrap_key`/`submit_event` now refuse. **A node whose database is recreated under an
>    existing key file needs `cairn-node establish-unwrap-key` before its first sealed write.** Never
>    make a red fixture green by weakening `ensure_unwrap_key`.
> 3. **`cairn-sync` LOADS its unwrap secret now (#503) — with ONE derived fallback, and the reason it is
>    safe is the reason not to widen it.** With no `<key>.unwrap` file AND a derived key that equals the
>    registered one, the daemon starts on the derived secret and warns on every startup (provably
>    pre-ADR-0066). A restored node derives a key that does NOT match and is refused — the #495 shape,
>    caught. Load-bearing asymmetry: an absent file may fall back, a present-but-unusable one (corrupt,
>    or no `CAIRN_KEY_PASSPHRASE`) never may, since a successful derive would mask the rot of the only
>    file carrying this node's custody off the machine. **Never simplify those two into one arm.**
>    Retiring the fallback is **#514**.
> 4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It adopts
>    a secret derived from the NEW signing seed and registers it, and `node_unwrap_key`'s singleton
>    registrar then refuses the real exported key permanently — `restore` warns about it explicitly, and
>    `submit_event`'s own refusal text names that command. Recover the export first;
>    `apply_local_state`'s refusal now explains the resulting state (the wrong registration, and the way
>    out is another restore into a fresh database) rather than surfacing a raw Postgres error.
> 5. **`init` now refuses a database that already has custody registered.** The file check
>    (`refuse_to_replace_existing_unwrap_key`) only fires when `<key>.unwrap` EXISTS, so a node that lost
>    its keystore could still run `init`, overwrite its signing key and mint a doomed custody key before
>    the registrar failed. `init` reads `node_unwrap_key` first now; the remedy it names is
>    `establish-unwrap-key`, idempotent — see trap 4 before running it on a restored node.

> [!WARNING]
> **[#511](https://github.com/cairn-ehr/cairn-ehr/issues/511) is CLOSED-as-COMPLETED on GitHub and
> nothing implements it — RE-OPENED and SEQUENCED 2026-08-31.** `grep -rn 'Secret32\|PublicKey32'
> crates/` still returns **nothing**. Its subject — every key in the custody plane is a bare `[u8; 32]`,
> so installing a *public* half as this node's secret compiles — is a live type-design hole the DR
> programme works directly inside. It is sequenced **after 2a and before 2c**: 2a's crate holds zero
> `[u8; 32]` (the newtypes would have cost 2a its "every call site compiled untouched" proof, an 83-site
> migration across four crates), while 2c/2d are where key material starts moving again. **Do not read
> the closed GitHub state as evidence the newtypes exist.**

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems: parts A
and B (authority floor + operator surface) are BUILT, enforcing nothing beyond display/emission; C+D are
DESIGNED and C1 is the next §5.9 BUILD — behind #500, which outranks it.** Read **ADR-0062/0063/0064/0065**
(`spec/decisions/`) before touching any of it; do not re-derive their decisions. The authority floor is ONE
predicate `cairn_claim_authority` (db/005) at exactly ONE site (db/048's `NOT EXISTS`), so display
coarsening, safety-rung emission and part C's dial all inherit it — it gives **#245** its first SQL
counterpart, not its mirror. Operator-surface §1.2 budget MET and pinned (residual **#436**).

**Parts C+D (ADR-0065; #377 merged, dependency REVERSED)** are a custody ladder — admission (default) →
named nodes → named actors — under one invariant: **narrowing changes the cost and noise of reading,
never whether content can be REACHED** (audited break-glass at every rung; rung-1 glass is a NETWORK act,
so a partitioned non-holder cannot reach it, **#498**). Node custody is the NORM, per-clinician the
EXCEPTION. Not to re-derive: the node's own DEK is the keyring and the floor is the glass (LOCAL); C and D
are NOT separable; custody is an additive field forcing composition to INTERSECTION, which can EMPTY
(**#499**); it narrows on `event`/`patient`, never `thread`; unparseable custody holds NOBODY while the
grade still STANDS. **C1** is rung 1 (`custody.nodes`, both doors, serve-door withholding) + audited
break-glass + the in-chart location signal; rung 2 is **#496** (blocked on a reader identity, §5.11);
chart-wide `patient` is OUT of C1, blocked on **#499**.

**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant
stands, so `cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new column
must be granted in db/049 §8 (`safety_read_grants.rs` names it), and that grant is cost-raising, not a
floor (**#425**, **#427** — never cite db/049 §8 as a confidentiality boundary; **#432** asks whether a
node should attempt one at all). Slice 65 follow-ons open: **#374** (thread resolution resolves only the
current head), **#378** (withdrawal rationale is clear text forever and replicates — the UI must warn
today), **#379** (grade in the twin), **#436** (**#374**/**#379** each need a DECISION, not a patch). The
`arrayref` incident (#445) is closed; residue **#454**.

> [!IMPORTANT]
> **Two code traps that outlive their slices, repeated here because both look like tidy-ups.**
>
> 1. **`content_address IS NOT NULL` is the "did anything win" test — never `subject_kind <> 'none'`.**
>    The catch-all arm reports `'coarsened'`, and `none` is a legal open-vocabulary value that collided
>    with the sentinel (ADR-0062 E6).
> 2. **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0 withholds
>    *reject power* (safe); in the sensitivity and safety ladders it would withhold *protection* or mute a
>    warning. Aligning them is the cleanup most likely to be attempted in good faith, and it reopens a
>    leak. **ADR-0065 adds a THIRD member that agrees for a DIFFERENT reason** (it withholds *quiet
>    access*, and break-glass keeps the content reachable) — do not carry that justification into a site
>    where reachability is not guaranteed.

**Three things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 time budget is a seeded
figure, not a measured one** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a
dated `TEMPLATE.md` copy; only the *write* half is measured (median 222 ms, **PARTIAL**), Slice 63 owes
both halves for registration (≤5s find, ≤20s register), write-cost half **#360** unwired, and db/044's
`gesture_kind` CHECK refuses a registration row until widened; (2) **the accessibility pass** — a live
VoiceOver run through the runbook's eight checks, keyboard-only
(`cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`), DOM
assertions automated by **#332**; (3) **make CI jobs REQUIRED status checks** (**#444**, admin-only —
"clippy + cargo test (cairn-gui)", "cargo doc (API surface)"), matching job names exactly, per
`CONTRIBUTING.md`'s dated table; (4) **#527's two Security-tab acts** — dismiss the 12 triaged
`cleartext-logging` alerts, THEN make `CodeQL` a fourth required check, in that order (see ⇒ NEXT).
**If a measurement falls outside its budget, that is the finding — file an issue, never adjust the
budget.**

**Other build candidates** (after #500; nothing blocks a choice): the **registration/search UI slice**
(the wrong-chart affordance paper has and the med-list window does not; per Slice 63 must **open** a
chart, never *retarget* one) · the **drugref term→anchor lookup** (the §9 advisory tier; closes the
coded↔uncoded case ADR-0059 decision 5 leaves open, needs a connection-model decision first,
`safety_class_map` its empty seam) · **the node/actor plane's two divergences** — db/007 fail-closes on
an unmappable type (**#301**), the clinical plane skips-and-advances instead (**#268**); neither is a
symmetric fix, both `loop:blocked`.

**Standing gate:** whole-project review cycles repeat periodically; no release for clinical use before
repeated cycles pass cleanly. Last full pass 2026-07-15 (#187–#217), fully closed; the runnable clinical
surface has never been through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
> session holds the main repo. **Never start it alongside one**: they contend on one cargo lock and one
> `test_serial_guard` advisory lock (a stray loop once stretched a session's suites ~3 → ~90 min). **A live
> IDE contends the same way** — rust-analyzer holds the shared `target/` lock, so a narrow `cargo test`
> blocks before it compiles, then times out. Fix is a scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the
> IDE; keeping one warm is also what makes the full gate ~15 min.

---

**Session date:** 2026-09-02 (**#527 — the CodeQL backlog**: the 18 critical alerts were a real defect, not the familiar false positive; renamed, guarded, house rule 6 corrected; opened #529) · previous: 2026-09-01 (**slice 2a review wave**: mutation audit 19/19 surviving → 18/18 killed; `health::assess` composed verdict; `Plane::Unknown`; `BackupError` taxonomy; #525 done) · previous: 2026-08-31 (**DR slice 2a — the shared medium format**: new crate `crates/cairn-medium` + `CAIRNB3`; closes nothing, #500 stays open; next is 2b; opened #522–#525, re-opened #511) · 2026-08-30 (**#503 — the shared keystore crate**: `cairn-sync` loads the node's provisioned unwrap key; federated sync restored; opened #514–#518, #520, #521) · 2026-08-24 (**DR slice 1** — the node unwrap key stops dying with the signing seed: #495 CLOSED, #500 still open; opened #503–#509, #511–#513; review wave also closed #502 item 2) · **Spec/ADRs:** v0.68 (ADR-0066; slice 2a adds no ADR/spec bump) · **`SCHEMA_GENERATION`:** 50 (`db/050`; slice 2a adds no migration — pure crate, no DB) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — orientation only; ROADMAP + the ADR log + git carry the detail. **Demographics slices
1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex/gender-identity · §4.3
address) and **the §5.2 advisory Python matcher** · **the §5.7 identity core C1–C5** (C5+ `reattribute`
waits on a clinical-note surface) · **the §5.4 John-Doe subsystem** (§5.12 push-alert open) · **the
§5.3/§5.8 search-before-create funnel** (ADR-0061; precedence rule #345 at db/005 step 8b) ·
**`clinical.medication` slices 1–6b** (ADR-0047/0048/0049/0050/0051/0059) under **born-sealed bodies**
(ADR-0052) and **per-write human authorship** (ADR-0053 — grading half-live until #245) · **the §5.9
stream complete through its read surface**, enforcing nothing beyond display/emission · **the med-list
node tier** (first clinical READ path + whole-list sign-off), **generic reprojection** (ADR-0057), the
**ADR-0056 admit-uninterpreted floor** and the **residual refusal contract** · **the L3 reference UI** —
`cairn-gui/`, a standalone workspace, one-way GUI → crates; the iced shell FAILED the accessibility bar
(spike 0004, retired 08-03), so today it is **`cairn-gui-tauri`** on one patient's medication chart (plain
JS, no npm), with the pane/routing/freshness state machine tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-09-02 (last) — #527: a discriminator is not a salt, and a scanner reads NAMES

**Closes nothing; #500 untouched. No ADR, spec bump or migration. Opened #529.** `main` carried **30
open CodeQL alerts**, the `CodeQL` check red on every PR run for weeks and **non-required** — so a
genuine critical would not have blocked a merge, and one was in there (#24, `format!("nonce-{}", "B")`
under a comment asserting it was runtime-derived; fixed on the #526 branch). Gate: 94 `cairn-medium`
tests unchanged, new guard 7/7, fmt + `clippy --all-targets -D warnings` clean.

1. **⇒ CodeQL PICKS ITS SINK BY THE NAME OF THE BINDING, and house rule 6's remedy does not touch that.**
   All 18 criticals were one per call site of two `cairn-medium` fixture helpers whose discriminator
   parameter was called `salt` — `salted_record(salt, n)`, `chain_of(n, salt)`. Nothing in that crate
   derives a key. Rule 6 says *compute it at runtime*; **both already did**, and it made no difference,
   because a derivation whose inputs are all literals is constant-folded straight through. The disproof
   sat in the same crate: `testkit::bytes(seed, …)` and `wire_pins::placeholder(seed, …)` run the
   **identical arithmetic** unflagged. **The only difference is the word.** Renamed to
   `distinct_record(lineage, n)` / `chain_of(n, lineage)` — **no fixture byte changed.** A legibility fix
   first: a reviewer who greps `salt` in a crate that seals bodies thinks they found a KDF.
2. **⇒ A TRIAGE TOOL THAT DROPS THE MESSAGE TURNS A DEFECT INTO NOISE.** `codeql-alerts.sh` printed the
   rule id and location but not `message.text`, so 18 alerts read `Hard-coded cryptographic value` —
   indistinguishable from the #146/#520 class dismissed twice before, and the obvious next step was to
   dismiss them again. With it they read *"This hard-coded value is used as a salt"*. **The rule id says
   which query fired; only the message says why.** Its allow rule was also never committed, so on a
   fresh clone the one tool that can read the list still prompted.
3. **Guarded, not just written down.** `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs` sweeps
   every shipping `src/` tree for a binding named exactly `salt`/`nonce`/`iv` and requires it in
   `ALLOWED` with the real construction named — 7 entries, 6 files. **Not a suppression list: the
   inventory of this tree's actual cryptography.** `#[cfg(test)]` inside `src/` is IN scope (all 18 were
   there). Carries a swept-file floor, 6 unit tests on its own matcher, and a **positive control** —
   cairn-keystore's Argon2id salt must still be found, or the matcher is broken rather than the tree
   clean. **CLAUDE.md house rule 6 gained the missing half.**
4. **The other 12 (`rust/cleartext-logging`, high) are all dismissable, for three different reasons** —
   per-alert verdicts in #527's comment: taint through a tuple/struct return (5 — the printed value is an
   address, a path or a count); **CLI receipts naming a patient** (3); test assertion text (4). Checked
   rather than assumed: **no daemon path prints a patient identifier** — but that holds by accident, not
   by rule, and `cairn-sync` carries CLI and daemon in one `main.rs`. **#529** filed: if a guard cannot
   state the boundary, the boundary is in the wrong place.

### 2026-09-01 — slice 2a's review wave: the format's guarantees, actually pinned (condensed)

**Closes nothing; #500 still open. No ADR, spec bump or migration.** A multi-agent review of PR #526 plus
a **mutation audit** found the suite tested the code against itself: **19 of 19 single-line mutations
survived**, several silently breaking every medium in the field. All fixed while the crate still had no
consumers — the cheapest this will ever be. 94 crate tests (was 51); audit re-runs **18/18 killed**.
**#525 done; #522/#523/#524 open as filed.** ROADMAP carries the per-item detail. What generalises:

1. **⇒ A round-trip cannot catch a MIRRORED change.** Plane tags, magic, `KIND_*`, chunk endianness,
   section field order and record flag bits could all be swapped **with the suite green**, because every
   test round-tripped through the same encoder/decoder pair. Only golden bytes fail — `src/wire_pins.rs`.
2. **⇒ Four false all-clears, one root cause: the honest facts and the verdicts lived on different
   types and nothing joined them.** An empty medium, a missing plane, a torn tail and a tampered record
   in the last unsigned segment all reported healthy. **`health::assess` is now the one composed
   verdict**; `sound()` and `carries_nothing()` are answered separately so neither stands in for the
   other; `intact()` → **`chain_intact()`** so a partial answer cannot read as a whole-medium one.
3. **⇒ A newer Cairn's plane read as DAMAGED.** **`Plane::Unknown(tag)`** is first-class now — keeps its
   records, chains normally, surfaces as a located fault. **`BackupError`** splits into
   `NotAMedium`/`UnsupportedByThisBuild`/`Damaged`: "upgrade this node" and "fetch another copy" are
   opposite remedies, and one opaque variant could make an operator discard a good medium mid-disaster.

### 2026-08-31 — DR slice 2a: the shared, two-plane, append-only medium format (condensed)

**Closes nothing — #500 stays open.** New crate `crates/cairn-medium` (today's `medium.rs` moved
verbatim, split by responsibility, the #503 pattern) plus **CAIRNB3**: CAIRNB2's head marker commits to
the whole sorted event set, so any append needs a full re-sign and rewrite; CAIRNB3 gives each
plane-tagged segment its own signed, chained attestation, so appending costs one signature.
CAIRNB1/CAIRNB2 still parse through untouched code (all 15 call sites compile unchanged). **⇒ One
global chain, not two per-plane ones** — `Segment.index` is the medium-wide file position, the only way
to catch a reorder or splice ACROSS planes; spec §7 implied per-plane numbering, corrected while
building. Spec/plan: `docs/superpowers/{specs,plans}/2026-08-31-dr-slice-2a-shared-two-plane-medium*.md`.

### 2026-08-30 — #503: the shared keystore crate, and federated sync comes back (condensed)

**Closes #503; opens #514–#518, #520, #521. No ADR, spec bump or migration.** New crate
`crates/cairn-keystore` (`CAIRNK1` sealed-bundle format + key-file loader + crash-safe atomic write,
moved verbatim out of `cairn-node`, whose **221** call sites compile untouched — the extraction's whole
proof), so `cairn-sync` can depend on the format without depending on a whole node application. It now
resolves its custody key **once at startup** through a pure decision table, replacing six independent
derivations. What generalises: **⇒ a guard that rejects a dead entry makes its own list a sequencing
constraint** (when it fails, delete the entry it names; never add one). **⇒ `cargo test --bin X
<filter>` compiles with `cfg(test)`, so new items look used** — use `--all-targets`. **⇒ deleting a
helper deletes its test's pin, and the pin may be the only one.** **⇒ a fail-open branch protected only
by a comment is protected by nothing** — if a comment warns readers off a change, a test should make
that change fail. **The gate's real cost is macOS, not cargo**: a cross-cutting change relinks ~134 test
binaries, ~6 hours, and a warm `target/` does not help.

### 2026-08-24 — DR slice 1: the unwrap key stops dying with the signing seed (condensed)

**Closed #495 (ADR-0066, spec v0.68) and #502 items 1–3; opened #503–#509, #511–#513. No migration.**
Shipped: an independent X25519 unwrap keypair in its own `<key>.unwrap` file; a lossless adoption path
for pre-ADR nodes (`keystore::adopt_derived_unwrap_secret` — the one place a node re-derives its old
secret, once); the secret and surviving custody rows riding the `CAIRNL1` export, a shredded event's DEK
excluded by construction; `restore` INSTALLING the inherited key instead of minting one. **#495's
status, #500's, and the five traps are in ⇒ NEXT — read that split before citing this anywhere.** Still
open: **#504** (a decision) · **#505** · **#506** · **#507** · **#508** (a container-format decision) ·
**#509** · **#512** · **#513**. **#511** is CLOSED-as-completed and does not exist in the tree — ⇒ NEXT.
What generalises: **⇒ the review wave found the slice's own failure shape inside the slice, twice** —
`restore` told an operator *"the export itself is intact; the code is what failed"* when it was not (a
precise untruth in the reassuring direction), and ADR-0066 decision 1 asserted what the branch's own
code contradicted: **the window in which an ADR is editable prose closes at merge.** **⇒ Breakage hid
from a gate three ways in one slice** — fail-fast masked 13 failures, `cargo test … | tail` masked the
exit status, and a cross-crate suite was invisible because `-p cairn-node` never builds it. **⇒ Four
defects were in the task briefs, not the implementations** — checked against what the code should do,
never against the gates the project runs. **⇒ Where no test carries the value across the disk, the one
link that matters is proven by nothing** — `#[serde(default)]` let a `skip_serializing` mutant
deserialize to `None`: every DR test green, every restore keyless. Mutation found it; the suite could not.

### 2026-08-23 → 08-20 — the DR audit, §5.9 part C, the misclassification cluster, the db-error sweep

**08-23, four passes.** Pass 4, the DR-guarantee audit, produced DR slice 1: confirmed #495, split #500
out, opened #502, added `dr_clinical_guarantee_gap.rs` (5 mutation-checked pins). Pass 3 — §5.9 parts
C+D (ADR-0065, spec v0.66→v0.67). Passes 1–2 — the misclassification cluster. **Still open:** #494 ·
#496 · #498 · #499 · #490 item 3 · #483 · #484 · #487 · #488 · #491 · #492 · #485 · #476.
**08-22 → 08-20, the db-error sweep.** Closed #460, #465, #467, #469, #471, #473–#475 (`db/050`, SCHEMA
49→50); #370, #457, #449–#453, #386, #381/#382/#385/#439, #446/#442/#443; opened #458. **Still open:**
#463 (a DECISION, overlay vs delete) · #464 · #458 · #470 · #447 · #327.

What binds: **⇒ a ceremony succeeding can be the worst shape of a bug** (an empty backup sealed and
reported success — every surface honest, the composite a precise untruth); **⇒ two defects that look
like one must be split when fixing either alone is useless** (#500 the bytes, #495 the key), and **where
a guarantee is already false, pin the defect, not the promise**; **⇒ a class is an operator
instruction** — the recogniser is a TYPE or `io::ErrorKind`, never message text; **⇒ a pin whose fixture
is built by the test leaves the production site unpinned**; **⇒ a line cap is never a reason to drop a
live issue** (a ROADMAP condensation once orphaned 22 in one edit). **⇒ `tokio_postgres::Error`'s
`Display` IS the string `"db error"`** — a bare kind match never chains to the source, and `LocalDbFault`
must not be "tidied" into an `anyhow!`, which silently reverts every local fault to `partition`. **⇒ a
frozen cursor looked exactly like a healthy cycle.** **⇒ the category test:** a sensitivity assertion IS
an event; `safety`/`clock_grade`/a rendition reference are FIELDS ON one, and refusing those forks the
event set (the **#342** trap). **⇒ a flag can be born on a re-apply**; a failed read reports `null`,
never `0`. **⇒ probe the family before fixing the member.** **A DB-free `cargo test` fails unless
`CAIRN_ALLOW_DB_SKIP=1`** (#450). Mechanics: force a write failure with a LOCK under a short
`lock_timeout`; `Debug` must delegate to `Display`; a VIEW checks the INVOKING user too.

### Older passes (Slices 61–69, 2026-08-02 → 08-20) — the lessons still worth holding

ROADMAP carries every slice in full. These are the ones a next session can still break.

1. **A guard defined over the list it guards is not a guard** (`assert_eq!(SubjectKind::ALL.len(), 3)`
   compared a constant to its own literal) — ask what INDEPENDENT source a guard checks against.
   **NAME, NEVER COUNT**: a count cannot separate custody-blind from genuinely empty.
2. **An optimisation removed a load-bearing redundancy and its comment asserted the opposite** — §11's
   bound is gated on the NEGATION of §10b's thread-free list. A wrong safety argument is worse than none.
3. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`**
   (ADR-0064 KNOWN GAP) — a mis-charted withdrawal reports effective: a reassuring-direction untruth on
   a confidentiality surface (**#436**).
4. **Two floor traps.** A pinned `search_path` must deny the temp schema the FIRST look — a decoy
   `event_log` made both write doors return SUCCESS while the INSERT landed in a temp table (**#430**,
   **#431**). A parameter name is not a security property — both key arguments are now `VerifiedKid`
   (**#428**).
5. **Slice 68** — the authority floor gates effect, never admission (the **#342** trap); computing the
   verdict at read cuts both ways (**#409**, **#408**/**#413**). PR #410: **7 of 11 production mutations
   survived a green suite**. `EXCEPTION WHEN OTHERS` does not catch a statement timeout (57014). Open:
   #413–#420, #422.
6. **Slices 66–67** — the seal boundary is the coarsening boundary: withhold the key, never the bytes.
   `safety_class_map` ships EMPTY. Open: **#406**, **#407**, #394–#402.
7. **Slices 61–63** — an attestation NAMES the displayed candidates, never counts them (**#360**); a
   unit-tested safety control can still be defeated by its calling surface; a compensating control
   outside CI is not a control (**#444**).

> [!IMPORTANT]
> **Two maintainer decisions to hold before any composite-clinical-object work.**
>
> **The loud failure belongs in the UI, not the floor** (2026-08-22, from #458): a defective attachment
> fails loud **in the UI** with **no blast radius for the rest of the clinical event** — validate before
> submit, fail at the attachment not at the save, no confirmation dialog (principle 3). The same decision
> refused a mandatory `descriptor` as a floor rule: **principle 4 forbids a required field satisfiable
> only by fabrication** — a rushed clinician types `x`, and an honest absence becomes a precise untruth.
>
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) —
> *a defect on one line never invalidates another*: the system may fail to record an order, but it may
> never cancel one.** Hold decision 2 (partial completion reported, never implied) and 7 (check the
> transaction boundaries).

**Repo conventions these runs learned the hard way:**
- **⇒ Three cargo trees; a new crate lands in all three lockfiles.** `extensions/cairn_pgx` and
  `cairn-gui` are `exclude`d from the root workspace but **ship anyway**, both depending on root crates
  **by path** — no root-workspace gate sees a stale sibling lockfile, only the `--locked` clippy run on
  those two trees does. **Workspace membership is a build-graph fact; "does it ship" is a different
  question** (#503, 2026-08-30).
- **A pinned COUNT lives beside the thing it counts, and a new member must be added to it.** The count
  failing IS the guard working — fix the list, and say in a comment why.
- **Guard before connect** — take `db::test_serial_guard(&base)` before `connect_and_load_schema`.
  **UUIDs bind as text** — bind `&uuid.to_string()`, cast in SQL as `$1::text::uuid`.
- **A second human actor needs a distinguishing determinant** — `actor_id` content-addresses the pinned
  determinant set, so two `{"role":"clinician"}` enrollments collide (ADR-0044/#152); use
  `enroll_human_with_role`.
- **`cargo test --lib` does not catch an import used only under `cfg(test)`** — use `--all-targets`.
- **A round-trip test cannot catch a MIRRORED format change.** Writer and reader move together and every
  assertion stays green; only a golden-byte fixture fails (2026-09-01, `cairn-medium/src/wire_pins.rs`).
- **⇒ A NAME is a scanner sink.** CodeQL flags a constant by the name of the binding it flows into, so a
  non-cryptographic value called `salt`/`nonce`/`iv` is a critical alert **per call site** and runtime
  derivation does not clear it. Reserve those three; read alerts with `scripts/codeql-alerts.sh` (the
  MESSAGE, not just the rule id) and never assume a finding is the familiar false positive (#527).

**Earlier sessions — condensed.** ROADMAP carries the per-slice detail (Slices 13–60, both tech-debt-loop
"Interlude" entries, every still-open issue). From Slice 60: **a refusal that persists nothing cannot be
audited**, and **when a call site cannot make a distinction, check whether a layer threw it away** (#480).
**GUI/L3 design threads (2026-07-16/18, design-only)** — detail in `scratch/ui-sketches/`; source
screenshots git-ignored under `docs/untracked_for_brainstorming/` — real photos, never commit or publish.

**Status of this file:** disposable scaffolding, **not** a source of truth; canonical docs win.
Regenerate each session, **under 500 lines** (#368) — *why* in the ADRs, *what* in the spec.

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not. **`docs/spikes/`** —
0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor, C1–C5 ✓ →
ADR-0029/0030); 0003 (Postgres on Android, G0–G3 ✓); 0004 (iced UI — FAIL on a11y → Tauri 2).
**`docs/case-studies/0001`**: 16 GP-software failure modes, all absorbed, **0 new architecture**.
**`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance. Code workspace: `/crates`
(`cairn-event`, `cairn-keystore`, `cairn-medium`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
`cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace); `poc/` is
frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS
  pinned to the trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate,
  genesis-stable `node_id`. Every honest gap declared at build time is closed **except the `localstate`
  clinical seams — half filled by slice 1** (custody travels; the clinical event log still does not —
  **#500**, the ⇒ NEXT warning); optional escrow rungs (Shamir/QR/TPM) remain. **Dual-identifier
  discipline** (ADR-0031) — the canonical plane (UUIDv7 + multihash) is the only identifier on the
  wire/in signed bodies; the projection plane may intern node-local `bigint` surrogates (`db/008` +
  leakage guard).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx`, self-serializing via a Postgres advisory
  lock (`db::test_serial_guard`). **Not "cluster-wide" — advisory locks are scoped PER DATABASE** (#467;
  ~124 comments still say otherwise, **#476**), so every caller takes the guard against `CAIRN_TEST_PG`.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels, `/techdebt-next` runs one fresh
  headless session per issue. Auto-merge ENABLED; works unattended (12 PRs); STOPPED by maintainer
  decision (⇒ NEXT). Live gaps **#326**, **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **DR slice 2b — #500 continues**, the transport seam + the paged pull; 2a (the shared medium format)
  landed 2026-08-31 and closed nothing; see ⇒ NEXT.
- **§5.9 parts C/D** (#232) — see ⇒ NEXT. Related: **#235** (shred authorization hooks), **#236** (FTS/RAG
  must build on `event_clear`).
- **`clinical.medication` — slices 1–6b DONE** (ADR-0059). Next: **drugref term→anchor lookup** (⇒
  NEXT); fuzzy/automatic reconciliation + a Tier-A dictionary; structured sig/frequency; correcting a
  dose event's effective date. Cross-cutting debt **#185**. Spine: `db/031`–`db/033`, `db/041`, `db/042`.
- **Demographics / matcher / identity — next slices** (`db/010`–`db/030` +
  `cairn-event::demographics`). B3-driven: gold set, locale packs, hub-tier duplicate sweep, proposal
  retraction. Identity: C5+ `reattribute` (waits on a clinical-note surface); §5.12 push-alert.
  Deferred **#168**, **#287**; rest in ROADMAP.
- **⇒ Test env — `scripts/run-db-gated-tests.sh` is the ONE command for the local gate** — the only one
  catching all three demonstrated hiding modes (fail-fast · a piped exit status · a cross-crate suite
  `-p <crate>` never builds): the `db/tests/*.sql` mirrors and the full workspace with
  `CAIRN_TEST_PG`/`PG2`/`PG3` baked in (PG18 + cairn_pgx on `127.0.0.1:5532`, DBs `cairn_test`/`2`/`3`);
  last full pass EXIT 0 over 139 binaries (2026-08-24; re-run 2026-08-30). **⇒ Cost depends on what
  changed, not target-dir warmth** — a warm `target/` does NOT make it ~15 min (#503's 6-hour surprise):
  a `Cargo.lock`/`cairn-node` change relinks ~134 binaries under macOS's one-time-per-binary Gatekeeper
  assessment, budget HOURS; a slice confined to one crate reruns almost nothing (`cargo test -p <crate>`,
  CI gates the rest — `-p cairn-sync` DOES build the cross-crate `clinical_pull.rs`). Without the three
  env strings the DB-gated suites self-skip and cargo counts them as passed, so since #450 a bare run
  FAILS unless `CAIRN_ALLOW_DB_SKIP=1` is declared. Mirrors are DESTRUCTIVE, refusing any DB lacking the
  `cairn_scratch_database` marker (#169). Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline
  pytest` (uv, never pip; gap **#314**, CI runs it). `clinical_pull` used to flake under a full-workspace
  run — #457 fixed the diagnostic, not the cause (still unnamed); `--test-threads=2` is the workaround.
- **Clinical case-mining** — historically the highest-signal generative mode; the primitives have absorbed
  every case so far without new architecture. Bring a real ED/hospital failure mode; record in
  [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation** (order/recall/referral with no closing ack), a named
  projection surfaced by salience not a modal; **③ impossible-vs-uncertain** for the in-DB floor.
- **Landing-page polish** — a non-developer page for the generated site (`web/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)):
  PASS twice. Remaining: fold the un-caveated B4 number into ADR-0015 to drop "provisional"; **#272**
  (reproject bench on the Pi rig).
- **easyGP session** — port [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s
  deferred items with live schema access (`rx!`/`tx!` parser + state machine; formulation/drug source +
  forced-manual rule table; prefetch warming daemon). Pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`.
  GUI-mining continues, opens the results/inbox design session (three-zone vs two-pane parked there).
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection
  per slice (§8.2 availability + windowing/resume shipped).

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** legal entity & jurisdiction — deferred until
  momentum/funding geography is clearer; trademark registration — principle recorded, instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session.** Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured.
