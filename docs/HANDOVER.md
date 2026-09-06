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
> - **◐ #500 — THE BYTES. THE WRITE HALF IS BUILT (DR slice 2c, 2026-09-06); THE READ HALF IS NOT, AND
>   #500 STAYS OPEN.** The medium is now a CAIRNB3 image carrying **both planes** — every `event_log` row
>   (clinical, demographic, identity, registration, erasure) with its **wrapped DEK** beside it — so the
>   record and the key that opens it finally leave the machine in one artifact. **Nothing yet RESTORES
>   one.** `restore` and `verify-backup` both read through `backup::node_plane_events`, which returns the
>   federation plane alone *on purpose*, and the carried `event_dek` + actor-registry rows are counted and
>   **not inserted** — there is no restored event for them to be custody of. So the sentence that matters
>   is UNCHANGED: a solo clinic backs up nightly, `verify-backup` passes, and a restore still recovers who
>   it peered with and **zero patients**. What changed is that the bytes now EXIST off-machine to be given
>   back; before, a dead disk was total loss and no later slice could have recovered it. **2d is the read
>   half and is what closes #500.** Both halves are pinned as siblings in
>   `dr_clinical_guarantee_gap.rs`: `medium_carries_both_planes` (a guarantee — it reddens if the capture
>   regresses) and `nothing_yet_restores_a_clinical_event_from_a_medium` (a **pin** — 2d reddens it, and
>   that is the guard working; **invert it then, never delete it**).
> - **⇒ #500 SPENT THREE DAYS *CLOSED ON GITHUB*, AND SIX OTHERS WITH IT (2026-09-04):** #101, #115,
>   #434, #441, #468, #500, #534, all reopened. GitHub reads `close`/`fix`/`resolve` **adjacent** to a
>   reference and never the sentence around it, so the sentences disclaiming the close performed it.
>   Now guarded by `scripts/check_closing_keywords.py` + `.github/workflows/closing-keywords.yml` (which
>   also prints what each merge WILL close); promotion to a required check is on **#444**. **The commit
>   convention `fix(#500):` is SAFE** — the parenthesis breaks the adjacency. Detail: the 2026-09-04
>   session entry.
> - **○ FOUR SLICES HAVE LANDED AND NONE OF THEM CLOSES #500.** **2a** (08-31, reshaped by its
>   review wave 09-01) is the FORMAT: `crates/cairn-medium` + **CAIRNB3**, per-segment signed chained
>   attestation; 19-of-19 surviving mutations became 18/18 killed, `health::assess` is the one composed
>   verdict, `Plane::Unknown(tag)` is first-class, `BackupError` splits three ways (**#522** loud,
>   **#523**/**#524** open, **#525** done). **2b** (09-02) is the SEAM and the PAGING:
>   `crates/cairn-wire`, `Transport`/`MediumTransport`, and a `do_pull` that pages and checkpoints EVERY
>   page — closing **#101 item 1 only** (items 2–3 keep #101 open); opened **#531**, **#532**,
>   **#534**–**#538**. **#511** (09-04) is the TYPES: `Secret32`/`PublicKey32` across four crates and the
>   `cairn_pgx` tree, so a PUBLIC half can no longer be installed as this node's SECRET custody key;
>   opened **#541**. **2c** (09-06) is the CAPTURE and the milestone: `db/051` gives the shred predicate
>   one in-DB home, `capture_plane` pages both planes onto CAIRNB3 (backfilling burned-`seq` gaps under a
>   bounded probe budget), the export gains the **actor registry** and a read-after-write, and
>   `verify-backup` refuses a stale or mismatched kit — closing **#522** and **#524**, fixing **#550**
>   in-branch, opening **#549**, **#551** and **#552**. **⇒ Next build is 2d**, the read half. Detail:
>   ROADMAP's 2a/2b/#511/2c entries.
> - **⇒ #527: READ THE ALERT LIST, DO NOT ASSUME IT.** `scripts/codeql-alerts.sh` prints it (read-only;
>   `gh api` is deny-listed repo-wide and must stay so). The critical 18 were a **REAL defect**, not the
>   #146/#520 false-positive class, and they are now **gone from `main`**. Measured 2026-09-04: **11 open,
>   all `rust/cleartext-logging`, all high, zero critical** — quote that only after re-running the script,
>   since it is the number this file has already been wrong about. **Two human acts still owed, IN THIS
>   ORDER:** dismiss the `cleartext-logging` alerts (per-alert verdicts in #527's comment), then make
>   `CodeQL` a REQUIRED check — a permanently-red required check trains everyone to merge past it, which
>   is how a genuine critical sat unread for a week.
> - **⇒ THE LOOSE END, NOW DECIDED.** 2c's answer to *"which carrier is authoritative for custody"* is
>   **BOTH**. The export is the OPTIONAL artifact (a passphrase-less cron run skips it with exit 0), so
>   custody there alone means tonight's medium beside a weeks-old export leaves every event sealed since
>   unreadable forever. The medium's copy is co-fresh with the events it unlocks; the export's is the
>   retroactively-filtered one; a restore reads a body if **either** still holds its key. **Its consequence
>   is deliberate, not a leak — see trap 7.**
>
> **A restored node therefore has a working key, and a medium full of bodies it does not yet read — 2c
> moved the bytes, not the reader.** Neither half is useful alone. **Never cite ADR-0026 decision 1's
> clinical promises as met** (its own text now carries dated errata E1/E2 saying exactly this, since
> the ADR's *"the medium carries no clinical event"* premise expired with 2c while every conclusion it
> drew from that premise still holds). Its promise 2 —
> *"node-default data-at-rest keys survive"* — has **no subject at all**: no node-default key tier exists,
> so it is neither honoured nor violated and must not be read as satisfied by anything slice 1 did.
> **[#502](https://github.com/cairn-ehr/cairn-ehr/issues/502) — items 1–3 fixed; item 4** (a discarded
> keystore-load reason) **stays open.** An unreadable export refuses the restore instead of being skipped
> in silence, a corrupt `.lsk` sidecar is diagnosed present-but-unusable rather than "absent", and
> `verify-backup` refuses a zero-event medium instead of printing an all-clear it had not established.
>
> **✓ FEDERATED SYNC WORKS AGAIN — [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503) IS CLOSED**
> (2026-08-30): `cairn-sync` LOADS the provisioned key at startup instead of deriving it at six sites —
> **one derived path survives by design**, trap 3.
>
> **The reusable lesson, and the reason #500 hid for weeks:** *a deferral is only honest while its stated
> precondition holds, and nothing in the repo watches for one expiring.* `localstate.rs`'s header declared
> its seam truthfully — *"the federation-node tier has no clinical surface yet"* — and ADR-0052 made that
> false without reopening it, while ROADMAP kept recording slices A–D as ✓ done. **Before trusting any ✓,
> check whether the sentence that justified it is still true.** Slice 2b's grep found SEVEN more of this
> shape in FOUR crates where memory said one; **#511 then found two more inside `seal.rs` itself**, still
> describing the identity↔custody coupling ADR-0066 had deleted eleven days earlier. **Grep, do not
> recall.**

> [!IMPORTANT]
> **Seven traps. Each is a step a next session takes in good faith.** (Five came from slice 1; trap 5 was minted by #511, trap 7 by DR slice 2c.)
>
> 1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY** — a pre-ADR-0066 node re-derives its old
>    secret exactly once, inside `keystore::adopt_derived_unwrap_secret`, keeping its `event_dek` rows
>    openable; **calling it anywhere else re-creates the #495 coupling.** Pinned by
>    `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`, which sweeps **every shipping tree**
>    (`sources::PRODUCTION_TREES` — `crates/`, `extensions/`, `cairn-gui/`, both `exclude`d trees) and
>    asserts every allow-list entry is still live, so it cannot quietly widen: **when it fails, delete the
>    entry; never add one.** The sweep and the test-gate matcher (`is_a_test_gate_attribute`, which must
>    recognise pgrx's `#[cfg(any(test, feature = "pg_test"))]`) move **together** — widening either alone
>    reddens the guard on correct code.
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
> 4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It adopts a
>    secret derived from the NEW signing seed and registers it, and `node_unwrap_key`'s singleton registrar
>    then refuses the real exported key permanently (`restore` warns explicitly; `submit_event`'s refusal
>    names the command). Recover the export first; `apply_local_state`'s refusal explains the resulting
>    state and that the way out is another restore into a fresh database.
> 5. **⇒ `Secret32` DOES NOT SEPARATE ONE SECRET ROLE FROM ANOTHER (#511, 2026-09-04).** An unwrap
>    secret, an Ed25519 signing seed and a DEK are all `Secret32`, so `Secret32::from_bytes(sk.to_bytes())`
>    compiles — that conversion IS the #495 coupling wherever it is not deliberate. **Do not take the
>    count from any comment: it is pinned in `crates/cairn-node/tests/secret32_conversions_are_named.rs`,
>    per file and by count.** Six production `Secret32::from_bytes` sites exist; exactly **two** turn the
>    signing seed into an unwrap secret — `keystore::adopt_derived_unwrap_secret` (the ADR-0066 migration)
>    and `cairn-sync`'s pre-ADR-0066 fallback (trap 3) — and the rest mint fresh CSPRNG output, seal the
>    seed AS the seed, or compare. A third of the first kind is the defect returning; the
>    newtypes make the PUBLIC-for-secret mix-up a compile error and nothing more. `unwrap_secret_is_the_signing_seed`
>    is still the only check for the secret-for-secret one, and `secret_opens_the_carried_custody` is
>    still the only proof a restored key OPENS anything — neither is made redundant by the types.
> 6. **`init` now refuses a database that already has custody registered.** The file check
>    (`refuse_to_replace_existing_unwrap_key`) only fires when `<key>.unwrap` EXISTS, so a node that lost
>    its keystore could still run `init`, overwrite its signing key and mint a doomed custody key before
>    the registrar failed. `init` reads `node_unwrap_key` first now; the remedy it names is
>    `establish-unwrap-key`, idempotent — see trap 4 before running it on a restored node.
> 7. **⇒ A BODY SHREDDED AFTER A CAPTURE KEEPS ITS DEK ON THAT MEDIUM. THAT IS NOT A LEAK — DO NOT "FIX"
>    IT (DR slice 2c, 2026-09-06).** It looks like an erasure that failed to propagate and it is the
>    definition of a backup: *"a backup is only a backup if it can restore the state of the system at the
>    time the backup was taken. Taking care of invalidated backups is a policy issue, not a core
>    enforcement one. The core will only guarantee availability and integrity of data"* (maintainer). At
>    the moment that medium was written the body WAS readable; a medium that dropped the key later would
>    report a state the node was never in, and could only do so by **rewriting a segment it has already
>    signed** — forfeiting the integrity guarantee that is the core's job. **Never filter old segments.**
>    (The mirror half is equally deliberate: a body shredded BEFORE its first capture never has its DEK
>    written, while its ciphertext still travels — a shred destroys the key, never the event.) Pinned by
>    `crates/cairn-node/tests/medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`, named
>    so nobody repairs it, with the framing and design §2.1 in its header. **What core does NOT do, and a
>    practice must be told:** completing an erasure across backups is **rotation** — capture fresh, destroy
>    old — and that interval IS the maximum time an erasure takes to complete across all copies. The
>    clinic's policy call, not Cairn's (principle 9; ADR-0005's *deletion is best-effort and declared*).
>    **2e's ADR owes that sentence in as many words.**

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems: parts A and B
(authority floor + operator surface) are BUILT, enforcing nothing beyond display/emission; C+D are DESIGNED and C1 is
the next §5.9 BUILD — behind #500's slice 2d, which outranks it.** Read **ADR-0062/0063/0064/0065** (`spec/decisions/`) before
touching any of it; do not re-derive their decisions. The authority floor is ONE predicate `cairn_claim_authority`
(db/005) at exactly ONE site (db/048's `NOT EXISTS`), so display coarsening, safety-rung emission and part C's dial
all inherit it — it gives **#245** its first SQL counterpart, not its mirror. Operator-surface §1.2 budget MET and
pinned (residual **#436**).

**Parts C+D (ADR-0065; #377 merged, dependency REVERSED)** are a custody ladder — admission (default) → named nodes →
named actors — under one invariant: **narrowing changes the cost and noise of reading, never whether content can be
REACHED** (audited break-glass at every rung; rung-1 glass is a NETWORK act, so a partitioned non-holder cannot reach
it, **#498**). Node custody is the NORM, per-clinician the EXCEPTION. Not to re-derive: the node's own DEK is the
keyring and the floor is the glass (LOCAL); C and D are NOT separable; custody is an additive field forcing
composition to INTERSECTION, which can EMPTY (**#499**); it narrows on `event`/`patient`, never `thread`; unparseable
custody holds NOBODY while the grade still STANDS. **C1** is rung 1 (`custody.nodes`, both doors, serve-door
withholding) + audited break-glass + the in-chart location signal; rung 2 is **#496** (blocked on a reader identity,
§5.11); chart-wide `patient` is OUT of C1, blocked on **#499**.

**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant stands, so
`cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new column must be granted in
db/049 §8 (`safety_read_grants.rs` names it), and that grant is cost-raising, not a floor (**#425**, **#427** — never
cite db/049 §8 as a confidentiality boundary; **#432** asks whether a node should attempt one at all). Slice 65
follow-ons open: **#374** (thread resolution resolves only the current head), **#378** (withdrawal rationale is clear
text forever and replicates — the UI must warn today), **#379** (grade in the twin), **#436** (**#374**/**#379** each
need a DECISION, not a patch). The `arrayref` incident (#445) is closed; residue **#454**.

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

**Three things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 time budget is a seeded figure, not a
measured one** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy; only the *write* half is measured (median 222 ms, **PARTIAL**), Slice 63 owes both halves for
registration (≤5s find, ≤20s register), write-cost half **#360** unwired, and db/044's `gesture_kind` CHECK refuses a
registration row until widened; (2) **the accessibility pass** — a live VoiceOver run through the runbook's eight
checks, keyboard-only (`cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`), DOM
assertions automated by **#332**; (3) **make CI jobs REQUIRED status checks** (**#444**, admin-only — "clippy + cargo
test (cairn-gui)", "cargo doc (API surface)"), matching job names exactly, per `CONTRIBUTING.md`'s dated table; (4)
**#527's two Security-tab acts** — dismiss the triaged `cleartext-logging` alerts (11 open as of 2026-09-04,
zero critical), THEN make `CodeQL` a fourth required check, in that order (see ⇒ NEXT). **If a measurement falls outside its budget, that is the finding — file an
issue, never adjust the budget.**

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
> IDE. **Do not read a warm target as a time estimate**: what the gate costs depends on what CHANGED (a
> cross-crate edit relinks every test binary), and the figures in this file have been wrong in both
> directions — #503 budgeted ~15 min and took six hours, #511 relinked the whole tree and finished in
> well under one. Measure the run you are in.

---

**Session date:** 2026-09-06 (**DR slice 2c** — the backup medium finally carries the clinical record, and nothing yet restores one; closed #522 and #524, fixed #550 in-branch, opened #549, #551 and #552; **#500 stays open for 2d**) · previous: 2026-09-04 (**the closing-keyword guard**: seven issues GitHub had closed that nobody closed — #101, #115, #434, #441, #468, #500, #534 — reopened, and a CI guard so the next negated sentence cannot do it again) and, earlier that day, **#511** (**the custody newtypes**: `Secret32`/`PublicKey32` across four crates plus the `cairn_pgx` tree; installing a PUBLIC half as this node's SECRET custody key is now a compile error; CAIRNL1 bytes unchanged and golden-pinned; opened #541) · 2026-09-02 (**DR slice 2b** — the transport seam and the paged pull; #101 loses only item 1; opened #531, #532, #534–#538) and, earlier that day, **#527** (the CodeQL backlog: the 18 criticals were a real defect, not the familiar false positive; opened #529, #530) · 2026-09-01/08-31 (**DR slice 2a** + its review wave; opened #522–#525, re-opened #511) · 2026-08-30 (**#503**, the shared keystore crate; opened #514–#518, #520, #521) · 2026-08-24 (**DR slice 1**: #495 CLOSED, #500 still open; opened #503–#509, #511–#513). Earlier sessions: see *Recent sessions* below. · **Spec/ADRs:** v0.68 (2c adds no ADR and no spec bump, but ADR-0066 gains dated errata **E1/E2** and the canonical `docs/spec/security.md` is corrected where 2c made it false) · **`SCHEMA_GENERATION`:** **51** (`db/051`, 2c's one migration — 2a, 2b and #511 added none) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — orientation only; ROADMAP + the ADR log + git carry the detail. **Demographics slices
1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex/gender-identity · §4.3
address) · **the §5.2 advisory Python matcher** · **the §5.7 identity core C1–C5** (C5+ `reattribute`
waits on a clinical-note surface) · **the §5.4 John-Doe subsystem** (§5.12 push-alert open) · **the
§5.3/§5.8 search-before-create funnel** (ADR-0061; precedence rule #345 at db/005 step 8b) ·
**`clinical.medication` slices 1–6b** (ADR-0047/0048/0049/0050/0051/0059) under **born-sealed bodies**
(ADR-0052) and **per-write human authorship** (ADR-0053 — grading half-live until #245) · **the §5.9
stream complete through its read surface**, enforcing nothing beyond display/emission · **the med-list node
tier** (first clinical READ path + whole-list sign-off), **generic reprojection** (ADR-0057), the
**ADR-0056 admit-uninterpreted floor** and the **residual refusal contract** · **the L3 reference UI** —
`cairn-gui/`, a standalone workspace, one-way GUI → crates; the iced shell FAILED the accessibility bar
(spike 0004, retired 08-03), so today it is **`cairn-gui-tauri`** on one patient's medication chart (plain
JS, no npm), pane/routing/freshness state machine tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-09-06 (last) — DR slice 2c: the medium finally carries the clinical record

**Closes #522 and #524; #550 opened and closed in-branch; opened #549, #551, #552, #553. #500 STAYS OPEN — 2c
is the WRITE half.** One migration (`db/051`, `SCHEMA_GENERATION` 50 → 51), no ADR and no spec bump, but
`docs/spec/security.md` corrected and ADR-0066 given dated errata E1/E2 where this slice made their
premise false. `cairn-node backup` now writes a CAIRNB3 medium carrying **both planes** — every
`event_log` row with its wrapped DEK beside the federation set; the `CAIRNL1` export carries the actor
registry and is verified after write; a stale or mismatched kit fails `verify-backup`. **Nothing reads a
clinical event back** — that is 2d. Full narrative in ROADMAP's 2c entry; what a next session needs:

- **⇒ TRAP 7 IS THE ONE TO READ FIRST** (⇒ NEXT): a body shredded AFTER a capture keeps its DEK on that
  medium, deliberately. It looks like a leak and is the definition of a backup. **Never filter old
  segments.**
- **⇒ POSTGRES BURNS AN IDENTITY VALUE BEFORE `ON CONFLICT` ARBITRATION, so permanent `event_log.seq`
  holes are ROUTINE on a federating node — and a watermark cursor silently loses an event at one.** A
  capture reading between two overlapping submits records the higher seq and advances past the lower;
  that event is then never captured by any future run *while the medium reports itself complete*.
  `cairn_medium::seq_gaps` existed for exactly this and had **zero callers**. Gaps are now backfilled
  NEWEST-first (oldest-first spends the whole budget on ancient burned holes an IDENTITY value can never
  refill and never reaches the one recoverable event) under `MAX_GAP_PROBES_PER_CAPTURE = 64`. Residual
  on **#549**. **The generalisable half: a monotonic cursor over an IDENTITY column is not a complete
  cursor, and nothing else in this tree checks that.**
- **⇒ A RULING WAS REVERSED MID-SLICE, AND THE REVERSAL IS THE RIGHT SHAPE.** Round 1 demanded exact
  legacy parity — a torn medium refuses. That is right for `verify-backup` (the health check) and
  **wrong for `restore`**: the torn remnant is already discarded by the parser, so refusing converts a
  partial loss into TOTAL loss in the one command that exists for the disaster where re-running the
  backup is impossible. `restore` now recovers the verified prefix with a loud warning and honest
  counts — **no confirmation dialog** (principle 3). The warning states the BRACKET rather than
  overclaiming: an interrupted append costs one increment, an indistinguishable partial COPY
  arbitrarily many.
- **⇒ A SAFETY PREDICATE WITH TWO SPELLINGS WAS ABOUT TO GET A THIRD.** *"A shredded body's key must not
  travel"* lived hand-written in two crates; 2c's capture would have been the third — the mirror-list
  class (#182, #404, #441) with a SAFETY predicate as the mirrored thing. `db/051` gives it one home in
  the floor. Its view carries `security_invoker = true`: a plain view reads as its OWNER and would have
  handed every role that can select it exactly the `event_dek` access db/037 refused — a decoy path
  around a floor that looks correct at its own site (the #430/#431 shape).
- **⇒ A CARGO FILTER OF THE FORM `--lib x:: --test A --test B` RUNS ZERO TESTS IN THE INTEGRATION
  TARGETS AND STILL EXITS 0.** The module-path filter is not scoped per target. **`EXIT=0` is not
  evidence a suite ran — the test COUNT is.** One `--test` per invocation.
- **§1.2 measured here** (first slice with a runnable capture): 10 000 fresh clinical events captured in
  **1.27 s** (budget < 60 s); an unchanged-log nightly capture in **0.86–1.10 s**, appending zero bytes
  (budget < 2 s). Both pass, no budget adjusted. The finding is **#552**: that time is linear in the
  WHOLE medium, so the 2 s budget is crossed at ~23 000 events — CAIRNB3's O(new-records) append property
  **stops at the `atomic_write` seam**, not "the rewrite grew".

**⇒ THE FINAL WHOLE-BRANCH REVIEW FOUND A CRITICAL THE TASK-SCOPED REVIEWS COULD NOT SEE, AND IT IS THE
SHARPEST LESSON OF THE SLICE.** Legacy succession — replacing a CAIRNB1/B2 medium with a CAIRNB3 one — is
the only destructive act in the backup ceremony, and nothing checked *whose* medium it was or whether the
successor held *as much*. The reachable disaster: a clinic's disk dies, the operator re-`init`s a node and
runs `backup --to` the USB holding their only medium **before** restoring. `read_self_node_id` returns
`None` on a not-yet-enrolled database, both captures read zero rows, and `assess()` on an EMPTY CAIRNB3
image is **vacuously sound** — chain intact, 0-of-0 records verified, no torn tail — so every guard passed
and `atomic_write` destroyed the medium, exit 0. Every component behaved correctly; the composite ate the
backup. That is #500's own shape, one layer up, inside the slice built to end it. Now refused by
`refuse_unsafe_legacy_succession` on either arm — a marker naming another node, or a successor holding
fewer `node_event` records — before the write, with the old medium left byte-identical. **Residual filed as
[#553](https://github.com/cairn-ehr/cairn-ehr/issues/553):** a CAIRNB1 medium has no marker at all, so a
*foreign* unmarked medium holding fewer events than this node still falls to the count arm alone. Also
note the count arm's premise — `node_event` is append-only — is enforced against DML only; **`TRUNCATE`
bypasses row triggers** and `db::reset_node_federation_tables` does exactly that in-tree.

**Three operator-visible behaviour changes 2c introduces, all deliberate, all able to fail a cron run that
previously always succeeded.** `backup` refuses an unsafe legacy succession and refuses to overwrite a file
at `--to` that is not a readable medium; `verify-backup` refuses a torn medium, a stale or mismatched DR
kit, an unsound medium (its own composed verdict now, not the federation plane alone) and a medium carrying
a plane this build cannot read; `restore`, by contrast, **recovers the complete verified prefix of a torn
medium rather than refusing it** — refusing there would convert a one-increment loss into total loss of an
operator's last copy. The asymmetry is the rule: *"we could not write a good medium"* and *"this kit cannot
restore"* page an operator; *"we wrote a good medium and something else was odd"* warns and exits 0. One
consequence worth knowing before it surprises someone: after a restore mints a new identity, a clinic's
pre-existing legacy medium can never again be succeeded **in place** — point `--to` at a new path.

### 2026-09-04 — seven issues GitHub closed that nobody closed (condensed)

**Reopened #101, #115, #434, #441, #468, #500 and #534. Closes no defect; builds one guard. No ADR, spec
bump, migration or DB change.** Found while checking, before starting 2c, that the tracking state ⇒ NEXT
rests on was real. It was not: **#500 — the issue this whole file is organised around — had been closed
on GitHub since 2026-09-01**, one second after PR #526 merged. Full narrative in ROADMAP.

1. **⇒ THE SENTENCE WRITTEN TO PREVENT THE OVER-CLAIM IS WHAT PERFORMED IT.** GitHub matches a closing
   keyword **adjacent** to a reference and never reads the sentence: *"It does **not** fix #500"*, *"It
   does close #101 **item 1**"*, *"**Filed rather than fixed:** #534"* each closed what it disclaimed.
   The #530/#511 stale-prose pattern with a twist — the prose was accurate; the *machine* read three
   words of it.
2. **⇒ A WRONGLY CLOSED ISSUE IS INVISIBLE, NOT WRONG-LOOKING.** Nothing surfaced any of the seven — not
   triage, not `/techdebt-loop`, not the ROADMAP prose still describing #441, #468 and #115's part 2 as
   open. **#115 sat closed for eight weeks.** The tell is timestamps: each closure is 1–3 s after a merge.
3. **⇒ THE GUARD HAD TO MIRROR GITHUB, NOT IMPROVE ON IT.** `fix(#500):` is **safe** — the parenthesis
   breaks the adjacency (proof: `fix(#288)`/`fix(#530)` on `main`, both open) — and a guard firing on
   nearly every commit here would be switched off within a week. `scripts/check_closing_keywords.py`
   reproduces GitHub's parser, then flags only a reference whose own clause denies it, tuned against 216
   merged PR bodies and 1650 commit messages. **Its false-positive shapes were found by running it over
   history, not by imagining inputs** (a qualifier belonging to the *next* sentence — `Closes #480.
   Partially addresses #490` — and a negation inside an em-dashed aside).
4. **⇒ THE REVIEW FOUND IT BLIND WHERE IT MATTERED, AND THE SAME SHAPE TWICE:** *the guard not looking at
   the text GitHub actually reads.* It could not see `(closes #N)` at all (one stray `(` in the
   lookbehind, added believing it protected `fix(#500):` — the real protection is `(` being absent from
   the separator), and it never scanned the **PR title**, which GitHub's merge commit carries as its
   body: `(closes #38)` in PR #42's *title* was that PR's only closing adjacency, and **#38 closed one
   second after the merge**. Four more followed (a parenthesised partial qualifier, negations matched as
   bare substrings, concatenated commit messages, and a range keyed off `base.sha`, which does not
   advance when `main` does). After all six: **the same 21 corpus flags, zero false positives, 236
   closing references instead of 227.** Its plumbing is `scripts/collect_pr_text.sh` with its own shell
   test — a checker fed the wrong text is not a control.
5. **Residual: the check is not required.** Promoting it is admin-only — **#444**, under #527's ordering
   rule: only promote a check that is green on `main`.

### 2026-09-04 (earlier) — #511: the custody newtypes (condensed)

**Closed #511. Did not touch #500 — no clinical event travels on any medium as a result. Opened #541,
and #543–#545 in review. No ADR, spec bump, migration or DB change.** New
`crates/cairn-event/src/keys.rs`: **`Secret32`** (Zeroizing inner, **redacting** `Debug`, constant-time
`PartialEq`) and **`PublicKey32`** (`Copy`, printable — published by design), re-exported from `seal`,
migrated across `cairn-event`, `cairn-keystore`, `cairn-node`, `cairn-sync` **and the `exclude`d
`extensions/cairn_pgx` tree**; all three lockfiles refreshed. ROADMAP's #511 entry carries the detail.
What outlives the slice:

1. **⇒ THE REVIEW ROUND CAUGHT THE SLICE COMMITTING THE DEFECT IT WAS FIXING.** Its headline finding was
   two `seal.rs` comments still asserting the coupling ADR-0066 deleted eleven days earlier (the #530
   pattern); five passes then found the same shape three times in its own output, worst a conversion
   count asserted in **six places silently counting three different populations** and naming a guard that
   counted none. **An inventory that lives in prose is a stale inventory waiting to happen** — the count
   now lives in `crates/cairn-node/tests/secret32_conversions_are_named.rs`, per file and by count, both
   its matcher and its bite mutation-tested. **Grep, do not recall.**
2. **⇒ THE TYPES CLOSE ONE CONFUSION, NOT THREE.** PUBLIC-for-secret is a compile error everywhere;
   **secret-for-secret is not** (trap 5 in ⇒ NEXT). `secret_opens_the_carried_custody` and
   `unwrap_secret_is_the_signing_seed` are **not** made redundant and must not be deleted as "covered by
   the types". Related: a refusal that moves earlier makes its old doc a lie about its own function —
   `recovered_unwrap_secret` became infallible and was rewritten, not left arguing for a check it no
   longer performs.
3. **⇒ THE WIRE PIN WAS TAKEN BEFORE THE TYPE MOVED, AND THAT ORDER IS THE WHOLE METHOD.**
   `localstate_wire_pins.rs` froze the exact `CAIRNL1` CBOR from the **pre-newtype** build and was
   mutation-checked before the migration began; still green = an existing off-site export still restores.
   (Settled in passing: ciborium writes `Vec<u8>` as a CBOR **array of uints**, not a byte string.)
4. **⇒ #541: NO CI JOB COMPILES `cairn_pgx`'s `pg_test` MODULE** — two `EventBody` fields behind and
   unbuilt for some time, because the pgx job's clippy step has **no `working-directory`** and so lints
   the ROOT workspace, which `exclude`s that tree. Fixed in passing; the gate gap is the issue. The #503
   "a suite invisible to the gate that was supposed to cover it" shape again.

### 2026-09-02 — DR slice 2b: the transport seam and the paged pull (condensed)

**Closed nothing; #500 still open. Closed #101 ITEM 1 only (items 2–3 keep it open). No ADR, spec bump
or migration. Opened #531, #532 and — from the final review — #534, #535, #536, #537, #538.** New crate
`crates/cairn-wire` (clinical-plane wire types, framing + its 64 MiB cap, the transport seam) lifted out
of `cairn-sync`'s binary-only `main.rs` so `cairn-node` can reach it in 2c/2d. **`Transport`** is the one
seam: `TcpTransport` is today's behaviour verbatim, **`MediumTransport`** is a CAIRNB3 medium answering as
a peer — which is what lets 2d's restore drive `cairn-sync`'s OWN puller (cursor, quarantine pen, custody)
against a file. `do_pull` **pages** (`DEFAULT_PAGE_EVENTS = 500`, `--page N`) and commits cursor +
quarantine floor after EVERY page; that per-page durability, not the smaller frame, is #101 item 1's
actual fix. ROADMAP's 2b entry carries the detail. The lessons that outlive it:

1. **⇒ THE PAGE CURSOR IS NOT THE CHECKPOINT CURSOR.** The next page is fetched from the last seq
   *received*; `max_seq` is the contiguous *handled* prefix and starts at the *committed* cursor. On a full
   sweep the two differ by the whole history below the cursor, so conflating them makes page 2 skip exactly
   what a sweep exists to reconcile.
2. **⇒ THE QUARANTINE FLOOR RULE HAS TWO HALVES, AND "COMPUTE IT OVER THE CYCLE" IS ONLY THE FIRST.** The
   PIN must be cumulative, **and the CLEAR must be earned**: `None` is a positive claim (*nothing is being
   withheld any more*), committed after every page. A clean page 1 cleared a floor at seq 900 and that
   penned clinical event was never re-offered, silently, with `skipped_unverifiable` at 0. A third route
   into the same clear — an empty page with a fabricated `seqs` array — closed it: **`complete` DEFAULTS TO
   THE UNCERTAIN DIRECTION** (an omitted flag costs a round trip, never a silent early stop with the cursor
   checkpointed as though the log were drained — principle 4 on a protocol field).
3. **⇒ STALE PROSE, THREE TIMES IN ONE SLICE.** SEVEN comments across FOUR crates asserted a deferral this
   slice retired (the load-bearing one: the correctness floor *"stops floor-ing exactly on the largest-history
   nodes"*) — and the write-up then said "three" because that number was *recalled*, in the very bullet whose
   moral is grep. Two more comments written inside the slice asserted properties their own code contradicted.
   **A seam invalidates every premise that named what was on the other side of it**:
   `chain_reaches_a_postgres_error` justified its `source()` walk with *"`do_pull` reaches its peer over a raw
   `TcpStream`"*, and one implementation now opens no socket. **Grep, do not recall.**
4. **⇒ THE UNCAPPED PAGE LOOP HAD A BOUND THAT DID NOT EXIST — and the wrong COMMENT was the defect.** It
   argued a flood is penned and the quota refuses, missing the two cheapest streams a peer can serve that pen
   nothing: events this node already holds, and bytes already penned. Either loops for ever, taking `cmd_run`
   with it — no pulls, no fingerprint, **no periodic full sweep**. Now bounded by a per-cycle budget that
   YIELDS (`budget_exhausted`), never refuses.
5. **`elapsed_ms` now measures a whole CYCLE, not one request** — `poc/walking-skeleton/harness/bet_a.py`
   feeds it into the A4 latency percentiles; **the harness's owner needs to meet this.**


### 2026-09-02 (earlier) — #527: a discriminator is not a salt, and a scanner reads NAMES (condensed)

**Closes nothing; #500 untouched. Opened #529, #530.** `main` carried **30 open CodeQL alerts** (11 as of 2026-09-04, none critical), the `CodeQL` check
red for weeks and **non-required** — and a genuine critical was in there (#24, `format!("nonce-{}", "B")` under a
comment asserting it was runtime-derived; fixed on the #526 branch). **⇒ CodeQL picks its sink by the NAME of the binding a value flows into**,
which house rule 6's *compute it at runtime* does not touch: a derivation whose inputs are all literals is
constant-folded straight through. All 18 criticals were one per call site of two `cairn-medium` fixture helpers whose
discriminator was called `salt`; renamed to `lineage`, **no fixture byte changed**. **⇒ A triage tool that drops the
MESSAGE turns a defect into noise** — `scripts/codeql-alerts.sh` prints it now (the rule id says which query fired;
only the message says why). Guarded by `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`: 7 `ALLOWED` entries
over 6 files, **the inventory of this tree's actual cryptography**, with a positive control. **⇒ Checking the guard's
own prose found a SECOND defect** — `PairingBundle.nonce` is signed into every offer and never read back by anything,
so the name promises replay protection that does not exist (**#530**, wants a DECISION, not a patch). The other 12
(`rust/cleartext-logging`) are all dismissable, per-alert verdicts in #527's comment; **#529** filed because *no
daemon path prints a patient identifier* holds by accident, not by rule. **Still owed, both HUMAN acts:** dismiss the
12, THEN make `CodeQL` a required check — in that order, because a permanently-red required check trains everyone to
merge past it. **#527 stays open.**

### 2026-09-01 / 08-31 — DR slice 2a and its review wave: the shared medium format (condensed)

**Closes nothing — #500 stays open. #525 done; #522/#523/#524 open as filed.** New crate `crates/cairn-medium`
(today's `medium.rs` moved verbatim, split by responsibility, the #503 pattern) plus **CAIRNB3**: CAIRNB2's head
marker commits to the whole sorted event set, so any append needs a full re-sign and rewrite; CAIRNB3 gives each
plane-tagged segment its own signed, chained attestation, so appending costs ONE signature, and CAIRNB1/CAIRNB2 still
parse through untouched code. **⇒ One global chain, not two per-plane ones** — `Segment.index` is the medium-wide file
position, the only way to catch a reorder or splice ACROSS planes; spec §7 implied per-plane numbering, corrected
while building. The review wave then found the suite testing the code against itself: **19 of 19 single-line mutations
survived**. **⇒ A round-trip cannot catch a MIRRORED change** — plane tags, magic, `KIND_*`, chunk endianness, section
field order and record flag bits could all be swapped with the suite green, because every test round-tripped through
the same encoder/decoder pair; only golden bytes fail, so `src/wire_pins.rs` pins them (re-run **18/18 killed**; 94
crate tests, was 51). **⇒ Four false all-clears, one root cause: the honest facts and the verdicts lived on different
types and nothing joined them** — an empty medium, a missing plane, a torn tail and a tampered record in the last
unsigned segment all reported healthy; **`health::assess` is now the one composed verdict**, and `intact()` →
**`chain_intact()`** so a partial answer cannot read as a whole-medium one. **⇒ A newer Cairn's plane read as
DAMAGED** — `Plane::Unknown(tag)` is first-class now, and `BackupError` splits
`NotAMedium`/`UnsupportedByThisBuild`/`Damaged`, because "upgrade this node" and "fetch another copy" are opposite
remedies and one opaque variant could make an operator discard a good medium mid-disaster. Spec/plan:
`docs/superpowers/{specs,plans}/2026-08-31-dr-slice-2a-shared-two-plane-medium*.md`.


### 2026-08-30 → 08-20 — the keystore crate, DR slice 1, the DR audit and the db-error sweep (condensed)

**#503 (08-30) — the shared keystore crate.** New `crates/cairn-keystore` (`CAIRNK1` format + loader +
crash-safe atomic write, moved verbatim out of `cairn-node`, whose **221** call sites compiled untouched — the
extraction's whole proof), so `cairn-sync` resolves its custody key **once at startup** through a pure decision table
instead of six independent derivations. Opened **#514**–**#518**, **#520**, **#521**. What generalises: **⇒ a guard
that rejects a dead entry makes its own list a sequencing constraint** (when it fails, delete the entry it names;
never add one) · **⇒ `cargo test --bin X <filter>` compiles with `cfg(test)`, so new items look used** (use
`--all-targets`) · **⇒ deleting a helper deletes its test's pin, and the pin may be the only one** · **⇒ a fail-open
branch protected only by a comment is protected by nothing**.

**DR slice 1 (08-24) — the unwrap key stops dying with the signing seed.** Closed **#495** (ADR-0066, spec v0.68) and
**#502** items 1–3; opened **#503**–**#509**, **#511**–**#513**. Shipped an independent X25519 unwrap keypair in its
own `<key>.unwrap` file, a lossless adoption path for pre-ADR nodes, the secret and surviving custody rows riding the
`CAIRNL1` export (a shredded event's DEK excluded by construction), and a `restore` that ADOPTS rather than mints.
**#495's status, #500's and the six traps are in ⇒ NEXT — read that split before citing this anywhere.** Still open:
**#504** (a decision) · **#505** · **#506** · **#507** · **#508** (a container-format decision; #511 narrowed it, did
not close it) · **#509** · **#512** · **#513**. **#511 closed 2026-09-04.** What generalises: **⇒ the review wave
found the slice's own failure shape inside the slice, twice** (**the window in which an ADR is editable prose closes
at merge**) · **⇒ breakage hid from a gate three ways in one slice** (fail-fast masked 13 failures, `cargo test … |
tail` masked the exit status, a cross-crate suite was invisible because `-p cairn-node` never builds it) · **⇒ four
defects were in the task BRIEFS, not the implementations** · **⇒ where no test carries the value across the disk, the
one link that matters is proven by nothing** (`#[serde(default)]` let a `skip_serializing` mutant deserialize to
`None`: every DR test green, every restore keyless — mutation found it, the suite could not).

**08-23 → 08-20 — the DR audit, §5.9 part C, the misclassification cluster, the db-error sweep.** Pass 4 (the
DR-guarantee audit) produced DR slice 1: confirmed #495, split **#500** out, opened **#502**, added
`dr_clinical_guarantee_gap.rs`. Pass 3 — §5.9 parts C+D (ADR-0065, spec v0.66→v0.67). Passes 1–2 — the
misclassification cluster. **Still open:** #494 · #496 · #498 · #499 · #490 item 3 · #483 · #484 · #487 · #488 · #491
· #492 · #485 · #476. **The db-error sweep** closed #460, #465, #467, #469, #471, #473–#475 (`db/050`, SCHEMA 49→50);
#370, #457, #449–#453, #386, #381/#382/#385/#439, #446/#442/#443; opened #458. **Still open:** #463 (a DECISION,
overlay vs delete) · #464 · #458 · #470 · #447 · #327.

What binds all three: **⇒ a ceremony succeeding can be the worst shape of a bug** (an empty backup sealed and reported
success — every surface honest, the composite a precise untruth) · **⇒ two defects that look like one must be split
when fixing either alone is useless** (#500 the bytes, #495 the key), and **where a guarantee is already false, pin
the defect, not the promise** · **⇒ a class is an operator instruction** — the recogniser is a TYPE or
`io::ErrorKind`, never message text · **⇒ a pin whose fixture is built by the test leaves the production site
unpinned** · **⇒ a line cap is never a reason to drop a live issue** (a ROADMAP condensation once orphaned 22 in one
edit) · **⇒ `tokio_postgres::Error`'s `Display` IS the string `"db error"`**, so a bare kind match never chains to the
source and `LocalDbFault` must not be "tidied" into an `anyhow!`, which silently reverts every local fault to
`partition` · **⇒ a frozen cursor looked exactly like a healthy cycle** · **⇒ the category test:** a sensitivity
assertion IS an event, while `safety`/`clock_grade`/a rendition reference are FIELDS ON one, and refusing those forks
the event set (the **#342** trap) · **⇒ a flag can be born on a re-apply**, and a failed read reports `null`, never
`0` · **⇒ probe the family before fixing the member** · **a DB-free `cargo test` fails unless
`CAIRN_ALLOW_DB_SKIP=1`** (#450). Mechanics: force a write failure with a LOCK under a short `lock_timeout`; `Debug`
must delegate to `Display`; a VIEW checks the INVOKING user too.

### Older passes (Slices 61–69, 2026-08-02 → 08-20) — the lessons still worth holding

ROADMAP carries every slice in full. These are the ones a next session can still break.
**A guard defined over the list it guards is not a guard** (`assert_eq!(SubjectKind::ALL.len(), 3)` compared a
constant to its own literal) — ask what INDEPENDENT source a guard checks against; **NAME, NEVER COUNT**, because a
count cannot separate custody-blind from genuinely empty. **An optimisation removed a load-bearing redundancy and its
comment asserted the opposite** — a wrong safety argument is worse than none. **`TargetState::OnAnotherChart` must
never collapse into `Held { still_standing: false }`** (ADR-0064 KNOWN GAP): a mis-charted withdrawal reports
effective, a reassuring-direction untruth on a confidentiality surface (**#436**). **Two floor traps:** a pinned
`search_path` must deny the temp schema the FIRST look — a decoy `event_log` made both write doors return SUCCESS
while the INSERT landed in a temp table (**#430**, **#431**); and a parameter name is not a security property, so both
key arguments are `VerifiedKid` (**#428**). **Slice 68** — the authority floor gates effect, never admission (the
**#342** trap); computing the verdict at read cuts both ways (**#409**, **#408**/**#413**); PR #410 had **7 of 11
production mutations survive a green suite**; `EXCEPTION WHEN OTHERS` does not catch a statement timeout (57014); open
#413–#420, #422. **Slices 66–67** — the seal boundary is the coarsening boundary (withhold the key, never the bytes);
`safety_class_map` ships EMPTY; open **#406**, **#407**, #394–#402. **Slices 61–63** — an attestation NAMES the
displayed candidates, never counts them (**#360**); a unit-tested safety control can still be defeated by its calling
surface; a compensating control outside CI is not a control (**#444**).

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
(`cairn-event`, `cairn-keystore`, `cairn-medium`, **`cairn-wire`**, `cairn-sync`, `cairn-node`,
`cairn-medication-view`, `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate
workspace); `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS
  pinned to the trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate,
  genesis-stable `node_id`. Every honest gap declared at build time is closed **except the `localstate`
  clinical seams — custody travels (slice 1) and the clinical event log now REACHES the medium (slice 2c),
  but nothing restores one — **#500**, the ⇒ NEXT warning); optional escrow rungs (Shamir/QR/TPM) remain. **Dual-identifier
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
- **⇒ DR slice 2d — #500 continues, and is the next build.** 2a (the format, 08-31), 2b (the transport
  seam + the paged pull, 09-02), **#511** (the custody newtypes, 09-04) and **2c** (the capture, 09-06)
  have all landed and none closed #500; **2d reads the clinical plane back off the medium** — the events,
  the carried `event_dek` rows and the carried actor registry — and is what closes it. See ⇒ NEXT. New
  from 2c: **#549** (a burned IDENTITY `seq` is indistinguishable from a lost clinical event; the
  probed-empty set wants a durable home and an operator surface), **#551** (the kit-restorability figure
  lives in a node-global file, not the kit — the same-path rotation case is still open) and **#552** (the
  nightly capture is O(whole medium), so the < 2 s budget is crossed at ~23 000 events). Still 2a's:
  **#523** (a corrupt section length under the cap is indistinguishable from a torn tail — named at its
  site in 2c, still `cairn-medium`'s to fix) and **#525**. New from #511:
  **#541** (no CI job compiles `cairn_pgx`'s `pg_test` module). From 2b: **#531** (decompose
  `cairn-sync/src/main.rs` — the older **#329** names the same file; a maintainer decision is wanted on
  which to keep) and **#532** (a multi-page pull that fails late reports nothing about the pages that
  landed). New from 2b's **final whole-branch review**: **#534** (a freeze stops content convergence, not
  just the cursor — wants a design decision), **#535** (`applied_addresses` unbounded per cycle), **#536**
  (an unopenable DEK is counted nowhere), **#537** (a failed pen release is permanent) and **#538** (the
  byte tier discards the new error taxonomy).
- **§5.9 parts C/D** (#232) — see ⇒ NEXT. Related: **#235** (shred authorization hooks), **#236** (FTS/RAG
  must build on `event_clear`).
- **`clinical.medication` — slices 1–6b DONE** (ADR-0059). Next: **drugref term→anchor lookup** (⇒
  NEXT); fuzzy/automatic reconciliation + a Tier-A dictionary; structured sig/frequency; correcting a
  dose event's effective date. Cross-cutting debt **#185**. Spine: `db/031`–`db/033`, `db/041`, `db/042`.
- **Demographics / matcher / identity — next slices** (`db/010`–`db/030` +
  `cairn-event::demographics`). B3-driven: gold set, locale packs, hub-tier duplicate sweep, proposal
  retraction. Identity: C5+ `reattribute` (waits on a clinical-note surface); §5.12 push-alert.
  Deferred **#168**, **#287**; rest in ROADMAP.
- **⇒ Test env — `scripts/run-db-gated-tests.sh` is the ONE command for the local gate**, the only one
  catching all three demonstrated hiding modes (fail-fast · a piped exit status · a cross-crate suite
  `-p <crate>` never builds): the `db/tests/*.sql` mirrors plus the full workspace with
  `CAIRN_TEST_PG`/`PG2`/`PG3` baked in (PG18 + cairn_pgx on `127.0.0.1:5532`, DBs `cairn_test`/`2`/`3`).
  **⇒ Cost depends on what changed, not target-dir warmth** (#503's 6-hour surprise): a cross-crate change
  relinks every test binary under macOS's one-time-per-binary Gatekeeper assessment — but #511 relinked
  the whole tree and still finished in well under an hour on a warm target, so **measure rather than
  budget from either figure**. A slice confined to one crate reruns almost nothing (`-p cairn-sync` DOES
  build the cross-crate `clinical_pull.rs`). Without the three env strings the DB-gated suites self-skip
  and cargo counts them as passed, so since #450 a bare run FAILS unless `CAIRN_ALLOW_DB_SKIP=1` is
  declared. Mirrors are DESTRUCTIVE, refusing any DB lacking the `cairn_scratch_database` marker (#169).
  Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest` (uv, never pip; gap **#314**).
  `clinical_pull` used to flake under a full-workspace run — #457 fixed the diagnostic, not the cause;
  `--test-threads=2` is the workaround.
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
