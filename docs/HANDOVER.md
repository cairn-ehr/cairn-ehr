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
>   **Its final review opened #522–#525. #522 and #523 are FORMAT decisions, cheapest NOW while no
>   CAIRNB3 medium exists:** no `chain_tail` helper, so two crates must each derive the next GLOBAL
>   chain index and agree (#522); and a corrupt section length UNDER the cap is indistinguishable
>   from a torn tail, whose remedies are opposite (#523). **#524** is 2c's to answer with the
>   custody-authority decision, not separately. **#525** is hygiene.
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
> keystore-load reason) **stays open.** A present-but-unreadable export now refuses the restore instead
> of being skipped in silence; a corrupt `.lsk` escrow sidecar is diagnosed present-but-unusable (*move
> it aside first*) rather than reported "absent"; `verify-backup` refuses a zero-event medium instead of
> `backup OK: 0/0` (checked in the CLI arm, not `all_intact()`), and stopped printing an all-clear it had
> not established: it reports the sealed export sibling and declares its contents were not checked.
>
> **✓ FEDERATED SYNC WORKS AGAIN — [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503) IS CLOSED**
> (2026-08-30, detail in Recent sessions below). `cairn-sync` briefly could not start against a node
> provisioned by `cairn-node init` (HKDF-derived vs. an independent unwrap key, ADR-0066); new
> `crates/cairn-keystore` carries the sealed key-file format both binaries need, and `cairn-sync` now
> **LOADS** the provisioned key at startup, resolved **once** and carried instead of derived
> independently at six sites. It refuses on divergence, on a restored node's fresh-seed derivation, and
> on a corrupt or passphrase-less key file — **one derived path survives by design**, see trap 3.
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
DESIGNED and C1 is the next §5.9 BUILD — behind #500, which now outranks it.** Read
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md),
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) and
[ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md) before touching any of it; do not
re-derive their decisions. The authority floor is ONE predicate `cairn_claim_authority` (db/005) at
exactly ONE site (db/048's `NOT EXISTS`), so display coarsening, safety-rung emission and part C's dial
all inherit it — it gives **#245** its first SQL counterpart, not its mirror. Operator-surface §1.2
budget is MET and pinned (residual **#436**).

**Parts C+D (ADR-0065; #377 merged, dependency REVERSED)** are a custody ladder — admission (default) →
named nodes → named actors — under one invariant: **narrowing changes the cost and noise of reading,
never whether content can be REACHED** (audited break-glass at every rung; rung-1 glass is a NETWORK
act, so a partitioned non-holder cannot reach it, **#498**). Node custody is the NORM, per-clinician the
EXCEPTION. Not to re-derive: the node's own DEK is the keyring and the floor is the glass (LOCAL); C and
D are NOT separable; custody is an additive field forcing composition to INTERSECTION, which can EMPTY
(**#499**); it narrows on `event`/`patient`, never `thread`; unparseable custody holds NOBODY while the
grade still STANDS. **C1** is rung 1 (`custody.nodes`, both doors, serve-door withholding) + audited
break-glass + the in-chart location signal; rung 2 is **#496** (blocked on a reader identity, §5.11);
chart-wide `patient` is OUT of C1, blocked on **#499**.

**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant
stands, so `cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new
column now requires granting it in db/049 §8 (`safety_read_grants.rs` names it), and that grant is
cost-raising, not a floor (**#425**, **#427** — never cite db/049 §8 as a confidentiality boundary;
**#432** asks whether a node should attempt one at all). Slice 65 follow-ons open: **#374** (thread
resolution resolves only the current head), **#378** (withdrawal rationale is clear text forever and
replicates — the UI must warn today), **#379** (grade in the twin), **#436** (**#374**/**#379** each need
a DECISION, not a patch). The `arrayref` incident (#445) is closed; residue **#454**.

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
assertions automated by **#332**; (3) **make two CI jobs REQUIRED status checks** (**#444**, admin-only —
"clippy + cargo test (cairn-gui)", "cargo doc (API surface)"), matching job names exactly, per
`CONTRIBUTING.md`'s dated table. **If a measurement falls outside its budget, that is the finding — file
an issue, never adjust the budget.**

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

**Session date:** 2026-08-31 (**DR slice 2a — the shared medium format**: new crate `crates/cairn-medium` + `CAIRNB3`; closes nothing, #500 stays open; next is 2b; opened #522–#525, re-opened #511) · previous: 2026-08-30 (**#503 — the shared keystore crate**: `cairn-sync` loads the node's provisioned unwrap key; federated sync restored; opened #514–#518, #520, #521) · 2026-08-24 (**DR slice 1** — the node unwrap key stops dying with the signing seed: #495 CLOSED, #500 still open; opened #503–#509, #511–#513; review wave also closed #502 item 2) · **Spec/ADRs:** v0.68 (ADR-0066; slice 2a adds no ADR/spec bump) · **`SCHEMA_GENERATION`:** 50 (`db/050`; slice 2a adds no migration — pure crate, no DB) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

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

### 2026-08-31 (last) — DR slice 2a: the shared, two-plane, append-only medium format

**Closes nothing — #500 stays open; full detail in the ⇒ NEXT warning above.** New crate
`crates/cairn-medium` (today's `medium.rs` moved verbatim, split by responsibility, the #503 pattern)
plus **CAIRNB3**: CAIRNB2's head marker commits to the whole sorted event set, so any append needs a
full re-sign and rewrite; CAIRNB3 gives each plane-tagged segment its own signed, chained attestation
instead, so appending costs one signature. CAIRNB1/CAIRNB2 still parse through untouched code — all 12
existing call sites compile unchanged. 51 tests green; `cargo build -p cairn-node` clean.

1. **⇒ One global chain, not two per-plane ones.** `Segment.index` is the medium-wide position in file
   order — one chain is what catches a reorder or splice ACROSS planes, which two independent chains
   could not. Spec §7 originally implied per-plane numbering; corrected while building.
2. **A fault is located, never merely counted.** Every `SegmentFault` carries its plane and index.

Spec: `docs/superpowers/specs/2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md`. Plan:
`docs/superpowers/plans/2026-08-31-dr-slice-2a-shared-two-plane-medium.md`.

### 2026-08-30 — #503: the shared keystore crate, and federated sync comes back

**Closes [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503); opens #514–#518, #520, #521. No ADR,
no spec bump, no migration — implements ADR-0066, decides nothing it left open.** New crate
`crates/cairn-keystore` (the `CAIRNK1` sealed-bundle format + key-file loader + crash-safe atomic write,
moved verbatim out of `cairn-node`), so `cairn-sync` can depend on it without depending on a whole node
application; `cairn-node` re-exports the three modules, so its **221** call sites compile untouched — the
extraction's whole proof. `cairn-sync` resolves its custody key once at startup through a pure decision
table (`src/unwrap_key.rs`) and threads a `NodeCustody` value down, replacing six independent derivations.

**Opened by its review wave (recorded 2026-08-31):** **#516** (extraction forced five `seal.rs`
AEAD/wrap helpers `pub(crate)` → `pub`) · **#517** (no test starts `cairn-sync` from a PROVISIONED key
file — every `clinical_pull.rs` scenario takes the fallback arm) · **#518** (the every-startup fallback
warning is printed by untested code, unpinning trap 3's "loud") · **#520** (7 CodeQL
`rust/cleartext-logging` false positives over X25519 public-key hex prefixes) · **#521** (`payload_ct`
lacks `serde_bytes`, so CBOR encodes it byte-per-element). What generalises:

**⇒ A guard that rejects a dead entry makes its own list a sequencing constraint** — the
`derive_unwrap_secret` allow-list entry was merely *reworded* at the plan's end, but the call MOVES file
mid-slice then disappears, so the guard would have failed twice; caught by a pre-flight plan scan, not a
test. When it fails, delete the entry it names; never add one. **⇒ `cargo test --bin X <filter>` compiles
with `cfg(test)`, so new items look used** — four items with no production caller stayed hidden until a
plain build (`warnings = "deny"` on `dead_code`) surfaced them; use `--all-targets`. **⇒ deleting a
helper deletes its test's pin, and the pin may be the only one** — removing `unwrap_key_matches` also
removed the only assertion that a divergence refusal names both keys; ask what a deleted test was
pinning, not just what it was testing. **⇒ a fail-open branch protected only by a comment is protected
by nothing** — extracted as the pure `registered_from_row` and tested; if a comment warns future readers
off a change, a test should make that change fail. **The gate's real cost is macOS, not cargo**: a
cross-cutting change relinks ~134 test binaries, ~6 hours, and a warm `target/` does not help — confined
slices run `cargo test -p cairn-sync` (which does build the cross-crate `clinical_pull.rs`; the
blindness the full gate exists to cover belongs to `-p cairn-node`).

### 2026-08-24 — DR slice 1: the unwrap key stops dying with the signing seed (condensed)

**Closed [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495) (ADR-0066, spec v0.68) and #502 items
1–3; opened #503–#509, #511–#513. No migration.** Shipped: an independent X25519 unwrap keypair sealed
in its own `<key>.unwrap` file; a lossless adoption path for pre-ADR nodes
(`keystore::adopt_derived_unwrap_secret`, the one place a node re-derives its old secret, once); the
secret and surviving custody rows riding the `CAIRNL1` export with a shredded event's DEK excluded by
construction; `restore` INSTALLING the inherited key instead of minting one. **#495's status, #500's,
and the five traps are in ⇒ NEXT — read that split before citing this anywhere.** Still open: **#504**
(dead `_node_sk` — a decision) · **#505** (a second recovery code) · **#506** · **#507** · **#508**
(unwiped CBOR copies — a container-format decision) · **#509** · **#512** · **#513**. **#511**
(`Secret32`/`PublicKey32`) is CLOSED-as-completed but **no such type exists in the tree** — see ⇒ NEXT.
What generalises:

**⇒ The review wave found the slice's own failure shape inside the slice, twice.** `restore` told an
operator *"the export itself is intact; the code is what failed"* after a spent recovery-code budget — a
precise untruth in the reassuring direction (damage and a mistyped code produce the identical `None`);
and ADR-0066 decision 1 asserted a property the branch's own code contradicted, scoped pre-merge since
ADRs are immutable after — the window in which an ADR is editable prose closes at merge. **⇒ Breakage hid
from a gate three distinct ways in one slice**: fail-fast masked 13 failures, `cargo test … | tail`
masked the exit status entirely, and a cross-crate suite was invisible because `-p cairn-node` never
builds it while `cargo check --workspace` compiles it without running it — use `--no-fail-fast`, never
pipe to `tail`, never accept `cargo check --workspace` as proof another crate's tests pass. **⇒ Four
defects were in the task briefs, not the implementations** — unformatted code, a gate invocation naming
one of three DB variables, a leaf shape that would leave a safety assertion permanently vacuous, and
unreachable control flow: the instructions were checked against what the code should do, never against
the gates the project actually runs. **⇒ Registering custody as a write-path side effect is what let the
defect hide** — making it provisioning turned six suites red. **⇒ Where no test carries the value across
the disk, the one link that matters is proven by nothing** — `unwrap_secret`'s `#[serde(default)]` let a
`skip_serializing` mutant deserialize to `None` silently, every DR test green and every restore keyless;
mutation found it, the green suite could not.


### 2026-08-23 (four passes) — the DR audit, §5.9 part C's design, and the misclassification cluster (condensed)

**Pass 4 — the DR-guarantee audit** produced DR slice 1: confirmed #495, split #500 out, opened #502,
added `dr_clinical_guarantee_gap.rs` (5 mutation-checked pins). **Pass 3 — §5.9 parts C+D designed**
(ADR-0065, spec v0.66→v0.67; opened #494–#496, answered #376, merged #377); **#494/#496/#498/#499** stay
open. **Passes 1–2 — the misclassification cluster** (closed #489, #482, #480, #490 items 1–2, #481,
#479, #477; opened #485, #487–#492). Still open: **#490** item 3 · **#483** (Windows-only, no CI
exposure) · **#484** · **#487** · **#488** · **#491** · **#492** · **#485** (23 files, 89 unnamed call
sites) · **#476** (~124 "cluster-wide" comments — actually per-database). What still binds: **⇒ a
ceremony succeeding can be the worst shape of a bug** (an empty backup sealed and reported success —
every surface honest, the composite a precise untruth); **⇒ two defects that look like one must be split
when fixing either alone is useless** (#500 the bytes, #495 the key), and **where a guarantee is already
false, pin the defect, not the promise**; **⇒ a class is an operator instruction** — the recogniser is a
TYPE or `io::ErrorKind`, never message text, and flattening a cause (`format!`/`anyhow!("…: {e}")`) is
worse than mis-classifying it; **⇒ a pin whose fixture is built by the test leaves the production site
unpinned**, `file!()` is the INCLUDING file's own path, and **a line cap is never a reason to drop a live
issue** (a ROADMAP condensation once orphaned 22 in one edit).

### 2026-08-22 → 08-20 — the db-error sweep, the freeze that hid, the flake that lied (condensed)

**Closed** #460, #465, #467, #469, #471, #473–#475 (`db/050`, SCHEMA 49→50); #370, #457, #449–#453,
#386, #381/#382/#385/#439, #446/#442/#443; **opened** #458. ROADMAP carries each pass in full. Still
open: **#463** (a DECISION, overlay vs delete) · **#464** · **#458** (non-object element — a loud UI,
not a floor rule) · **#470** · **#447** · **#327**. Lessons: **⇒ `tokio_postgres::Error`'s `Display` IS
the string `"db error"`** — a bare kind match never chains to the source; `LocalDbFault` is not a
rendering and must not be "tidied" into an `anyhow!`, which silently reverts every local fault to
`partition`. **⇒ a frozen cursor looked exactly like a healthy cycle** (`pull_into`'s freeze paths all
`break`/`Ok`, so a `53100` disk-full emitted neither `LOCAL FAULT` nor `PARTITION`). **⇒ the category
test:** a sensitivity assertion IS an event; `safety`/`clock_grade`/a rendition reference are FIELDS ON
one, and refusing those forks the event set (the **#342** trap) — db/027 raises where db/050 records,
and `WHEN OTHERS` there would write a disk error into the ledger as peer garbage. **⇒ a flag can be born
on a re-apply** (keyed on admitted addresses + a `flag_id` watermark); a failed read reports `null`,
never `0`; peer text is not display text. **⇒ probe the family before fixing the member** — #370 named
one field, measured nine freeze paths across four SQLSTATE classes plus four silent-wrong ones; refuse
what FAILED or was silently WRONG, accept what worked. **A DB-free `cargo test` fails unless
`CAIRN_ALLOW_DB_SKIP=1`** (#450); an opt-out must read an unrecognised value as NOT permission.
Mechanics: force a write failure with a LOCK under a short `lock_timeout`, never a trigger/`REVOKE`;
`Debug` must delegate to `Display`; a VIEW checks the INVOKING user too; `git check-ignore` needs
`--no-index` and has three exit codes (0/1/128).


### Older passes (Slices 61–69, 2026-08-02 → 08-20) — the lessons still worth holding

ROADMAP carries every slice in full. These are the ones a next session can still break.

1. **A guard defined over the list it guards is not a guard** (`assert_eq!(SubjectKind::ALL.len(), 3)`
   compared a constant to its own literal) — ask what INDEPENDENT source a guard checks against; where a
   family has an authoritative list, read the list (`proacl`'s NULL ACL is the PERMISSIVE case).
   **NAME, NEVER COUNT**: a count cannot separate custody-blind from genuinely empty; a union view whose
   arms mean opposite things must never get one summary sentence.
2. **An optimisation removed a load-bearing redundancy, and its comment asserted the opposite** — §11's
   bound is gated on the NEGATION of §10b's thread-free list; widening it leaves a standing
   `sequestered` grade reading back `('routine','none')`. A wrong safety argument is worse than none.
3. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`** (ADR-0064's
   KNOWN GAP) — `cairn_sensitivity_standing` is patient-scoped on both sides, so a mis-charted
   withdrawal reports effective: reassuring-direction untruth on a confidentiality surface (**#436**).
4. **Two floor traps.** A pinned `search_path` must deny the temp schema the FIRST look — a decoy
   `event_log` made both write doors return SUCCESS while the INSERT landed in the caller's temp table
   (**#430**, **#431**). A parameter name is not a security property — both key arguments are now a
   `VerifiedKid` newtype (**#428**).
5. **Slice 68** — the authority floor gates effect, never admission, only withholding (the **#342**
   trap); computing the verdict at read cuts both ways (**#409**, **#408**/**#413**). PR #410: 7 of 11
   production mutations survived a green suite. `EXCEPTION WHEN OTHERS` does not catch a statement
   timeout (57014). Open: #413–#420, #422 (**#415** — expect noise).
6. **Slices 66–67** — the seal boundary is the coarsening boundary: withhold the key, never the bytes;
   read coarsening is a rendering choice, not a floor. `safety_class_map` ships EMPTY. Open: **#406**,
   **#407**, #394–#402.
7. **Slices 61–63** — an attestation NAMES the displayed candidates, never counts them (**#360**); a
   unit-tested safety control can still be defeated by its calling surface; a compensating control
   outside CI is not a control (**#444**).

> [!IMPORTANT]
> **Two maintainer decisions to hold before any composite-clinical-object work.**
>
> **The loud failure belongs in the UI, not the floor** (2026-08-22, from #458): a defective attachment
> fails loud **in the UI** with **no blast radius for the rest of the clinical event** — validate before
> submit (the door refusing the whole event is a backstop that should never fire), fail at the
> attachment, not at the save, no confirmation dialog (principle 3). The same decision refused a
> mandatory `descriptor` as a floor rule: **principle 4 forbids a required field satisfiable only by
> fabrication** — a rushed clinician types `x`, and an honest absence becomes a precise untruth.
>
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) —
> *a defect on one line never invalidates another*: the system may fail to record an order, but it may
> never cancel one.** Hold decision 2 (partial completion reported, never implied) and 7 (check the
> transaction boundaries).

**Six repo conventions these runs learned the hard way:**
- **⇒ Three cargo trees; a new crate lands in all three lockfiles.** `extensions/cairn_pgx` and
  `cairn-gui` are `exclude`d from the root workspace but **ship anyway**, both depending on root crates
  **by path** — no root-workspace gate sees a stale sibling lockfile, only the `--locked` clippy run on
  those two trees does. Refresh both whenever the root workspace gains or loses a crate: **workspace
  membership is a build-graph fact; "does it ship" is a different question** (#503, 2026-08-30).
- **A pinned COUNT lives beside the thing it counts, and a new member must be added to it** — e.g. a new
  `cairn_decode_hex_or_raise` call site in `hex_decode_helper.rs`'s per-file list. The count failing IS
  the guard working — fix the list, and say in a comment why.
- **Guard before connect** — take `db::test_serial_guard(&base)` before `connect_and_load_schema`.
  **UUIDs bind as text** — `cairn-node` does not enable `with-uuid-1`; bind `&uuid.to_string()`, cast in
  SQL as `$1::text::uuid`.
- **A second human actor needs a distinguishing determinant** — `actor_id` content-addresses the pinned
  determinant set, so two `{"role":"clinician"}` enrollments collide (P0001, ADR-0044/#152); use
  `enroll_human_with_role`.
- **`cargo test --lib` does not catch an import used only under `cfg(test)`** — use `--all-targets`.

**Earlier sessions — condensed.** ROADMAP carries the per-slice detail (Slices 13–60, both tech-debt-loop
"Interlude" entries, every still-open issue). From Slice 60: **a refusal that persists nothing cannot be
audited**, and **when a call site cannot make a distinction, check whether a layer threw it away** (#480).
**GUI/L3 design threads (2026-07-16/18, design-only)** — detail in `scratch/ui-sketches/`; source
screenshots git-ignored under `docs/untracked_for_brainstorming/` — real photos, never commit or
publish. Headline: easyGP's six editing-area invariants ≅ Cairn's event envelope (ADR-0021's seam).

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
  decision — see ⇒ NEXT. Live gaps: **#326**, **#312**, **#322**.

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
- **Demographics / matcher / identity — next slices** (`db/010`–`db/030` + `cairn-event::demographics`).
  Next (B3-driven): a large gold set; locale comparator packs; hub-tier duplicate sweep; proposal
  retraction. Identity: C5+ `reattribute` (waits on a clinical-note surface); §5.12 push-alert. Deferred
  **#168**, **#287**; rest in ROADMAP.
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

- **Parked (don't re-litigate without new reason):** stewarding legal entity & jurisdiction — deferred
  until momentum/funding geography is clearer; formal trademark registration — principle recorded
  (stewardship doc), legal instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session** — working conventions,
  the twelve founding principles, the §9 defect-blast-radius language rule. Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr`
  org; `cairn-ehr.org`+`.com`; PyPI/crates.io/npm `@cairn-ehr` placeholders).
