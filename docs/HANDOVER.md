# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ THE DISASTER-RECOVERY HOLE IS CLOSED. #495, #500 AND #554 ARE ALL SHUT — AND THE SCOPE
> BELOW IS PART OF THE CLAIM, NOT A FOOTNOTE ON IT.**
>
> A solo clinic can now lose its disk, restore from the medium plus its export, and **open a
> chart**. The three halves that had to land are all in: the KEY (#495, ADR-0066, 2026-08-24),
> the BYTES' write half (#500, DR slice 2c, 2026-09-06), and the READ half
> ([#554](https://github.com/cairn-ehr/cairn-ehr/issues/554), DR slice 2d, 2026-09-10,
> [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), spec **v0.69**).
>
> **What "closed" does and does not mean.** A record a restore cannot apply is **quarantined
> with its custody**, not dropped, and the restore exits **non-zero** saying so. The actor
> registry re-enters on the export container's AEAD alone — the one part of a restore that is
> **not** verify-on-apply, accepted deliberately and printed to the operator. And **rows and
> custody coming back is not the same claim as a body opening**: a double-wrapped `event_dek`
> row is present, well-formed and exactly the right length, so the test to cite is
> `restore_reads_the_clinical_plane.rs`, which decrypts a real sealed body back to its twin
> text — not the row counts in `dr_clinical_guarantee_gap.rs`.
>
> **The pin inverted rather than being deleted.**
> `nothing_yet_restores_a_clinical_event_from_a_medium` is now
> `a_clinical_event_restores_from_a_medium`. Its **leg 1** (`node_plane_events(&image) ==
> federation`) did NOT invert and must not be made to: 2d added `clinical_plane_records`
> **beside** that reader rather than widening it, which is what keeps a federation record's
> fate independent of a clinical segment's chain.
>
> **⇒ BOTH THINGS 2d OWED ARE DISCHARGED (2026-09-10, PR #573). NEITHER CHANGED PRODUCT
> BEHAVIOUR: ONE RECORDED A DECISION, THE OTHER MEASURED ONE.**
>
> **1. §5.2's `Provenance` ruling — DECIDED, [ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md), spec v0.70, closes #571.**
> **Provenance warns; it never gates, on either plane.** The shipped `eprintln!` arms were
> already the answer. Four reasons, and do not re-derive them: refusing converts a partial loss
> into a **total** one (the ruling already made for a torn tail, an `Unknown` plane, a legacy
> medium); a gate does not buy what it looks like — per-event signatures stop **forgery, not
> omission**, so a prompt ratifies an **identity**, never a **record set**; principle 3 forbids
> the mechanism by name; and **input is not ceremony** — the recovery-code prompt asks for a
> secret only the operator holds, a provenance confirmation asks them to ratify a judgment the
> machine already printed. Design test 18 is **restated, not dropped**, accepted wording left
> struck through above it. ⚠️ **Recorded as a NEW ADR, not the ADR-0067 erratum #571 asked for**:
> the errata rule needs a passage factually false *about the code* to sit above the correction,
> and ADR-0067 never mentions provenance; decision-shaped content takes a new ADR.
>
> **2. The §1.2 measurement — MEASURED, and it PASSES with ~5× headroom.**
> **100 003 events restore in 116.7 s against a 600 s budget; 85 000 sealed bodies open on the
> restored node.** Linear at **1.17 ms/event, no bend**, so the ceiling is predictable rather
> than a cliff: 600 s arrives near **510 000 events** on an M3 Max. Rig
> `scripts/measure_dr_restore.py`; write-up `crates/cairn-node/results/2026-09-10-macos-m3max.md`;
> runbook + template beside it. Verified past the summary line (85 000 `event_dek` / `event_clear`
> / `medication_statement`, plus a twin read back) — **rows arriving is not a body opening.**
>
> **⇒ `M > N` STILL STANDS AND #512 STAYS OPEN — BUT THE EXCESS ACT CHANGED IDENTITY, AND THE
> NEW ONE IS BETTER-POSED.** ADR-0068 deleted the provenance confirmation DR slice 1's plan
> blamed for `M = 3`. The third act is real anyway and the plan mis-assigned it: its claim that
> the escrow secret and the invocation are *"one interactive ceremony"* is **false** — the old
> node's recovery code is a **second, separately-prompted secret**, asked for after the node
> plane is already applied. Progress, not a lateral move: an identity confirmation has no paper
> counterpart and could never be bundled, a second prompt plausibly can, which is what keeps
> `K = 2` believable.
>
> **⇒ #572 IS NEW AND IS THE SHARPEST THING THIS RUN FOUND: A RESTORE CANNOT BE SCRIPTED AT
> ALL.** The recovery code is read through `rpassword` (fails on any non-tty) and has **no flag
> and no env var**, unlike the new key's passphrase. A piped code does not merely fail — the read
> errors, the export never opens, and the restore recovers **ZERO PATIENTS** while exiting
> non-zero. So no cron DR drill, no scripted rehearsal, and the measurement rig has to allocate a
> pty purely to work around it. Needs a **decision** (flag / file / env / explicit
> `--non-interactive` refusal), and #527's *"no cron-run command reaches `print_recovery_code`"*
> triage would need re-checking by whichever lands.
>
> **⇒ #552 IS CONFIRMED WITH A NUMBER IT DID NOT HAVE.** A single capture is linear
> (~0.15 ms/event); the cost that bites is the one that never goes away. **A second capture of
> the same 100 003-event medium with ZERO new events cost 9.95 s**, against 15.0 s for the one
> that wrote all of them — about **two thirds of a nightly capture is independent of how much is
> new**. Its ~23 000-event estimate for crossing a 2 s budget is if anything optimistic;
> interpolation puts it near **14 000**.
>
> **⇒ THE STRONGEST REMAINING DR CANDIDATE IS THE §7 TEST DEBT** — it is what 2d explicitly
> declined to claim, and it now bundles cleanly with three of the issues 2d filed: **#568**
> (`do_requeue`'s custody-carrying arm — **the remedy every penned reason advertises** — has zero
> tests; every call site passes `None`), **#570** (the restore CLI surface is untested, exit code
> included) and **#567** (`verify-backup`'s OK is still federation-only, so a green verify says
> nothing about the plane a restore now applies — arguably the sharpest safety gap left on this
> path). Note **#570 and #572 are the same wall**: a CLI test cannot drive the recovery-code
> prompt without a pty either, which is one reason that surface has no tests.
>
> **Eight §7 tests are unwritten** — the behaviour is built and green, the pins are not:
> 4 (custody survives the pen → requeue → the body opens), 7 (the pen uncapped **at a volume
> above the row cap**, which the design demands explicitly because a handful of events passes
> against the unfixed quota), 14 end-to-end through the CLI, 16 (duplicate `source_seq`
> no-op / substitution refused), 17, 19 behaviourally (the mid-restore-crash re-restore), 22
> and 23. **PR [#566](https://github.com/cairn-ehr/cairn-ehr/pull/566) carries the table.** The
> design's own standard is that *"a decision in §2–§6 with no entry here is a decision this
> slice is not entitled to claim"* — so 2d does not claim them.
>
> **⇒ 2e IS RETIRED AS A LABEL** (ADR-0067 took its ADR and its spec bump). What was under the
> name is operational and lives on its own issues: **#551** (the kit-restorability figure has
> no per-kit home; a same-mount-point rotation still false-greens) and **#553** (an unmarked
> foreign legacy medium can still be destroyed by succession). Do not leave "2e" standing as an
> empty container — that is how future sessions defer into one.
>
> **Still broken after 2d, all named rather than assumed away:** **#549** (a burned identity
> `seq` is indistinguishable from a lost clinical event; 2d makes the consequence visible via a
> legible pen reason and **re-defers** the `seq_gaps` operator surface — recorded, not dropped)
> · **#552** (a capture is O(whole medium), and the read side adds parses at the same seam;
> **read-side peak memory is unbudgeted** and a Pi or Android node is a legitimate restore
> target, so streaming stays deferred here) · **#536** (an unopenable DEK is counted on the
> RESTORE path only; the sync half is open) · **#502 item 4** (a discarded keystore-load
> reason) · **#101 items 2–3** · **#512** (`M > N` stands; the TIME half is now measured and
> passes — see above) · **#572** (**new** — no non-interactive path for the recovery code, so a
> restore cannot be scripted or rehearsed by cron; needs a decision, not a patch).
>
> **Never cite ADR-0026 decision 1's promise 2** — *"node-default data-at-rest keys survive"* —
> as met by any of this. It has **no subject at all**: no node-default key tier exists, so it is
> neither honoured nor violated, and ADR-0067 says so in as many words.

> [!WARNING]
> **⇒ #527: READ THE ALERT LIST, DO NOT ASSUME IT.** `scripts/codeql-alerts.sh` prints it
> (read-only; `gh api` is deny-listed repo-wide and must stay so). The critical 18 were a
> **REAL defect**, not the #146/#520 false-positive class, and are gone from `main`. Measured
> **2026-09-10 (re-run, this is the current figure): 10 open, all `rust/cleartext-logging`, all high,
> zero critical, all on `refs/heads/main`** — one fewer than the 11 measured 2026-09-04. Quote that
> only after re-running the script, since it is the number this file has already been wrong
> about. The open set is now **#5, #6, #7, #8, #13, #15, #16, #20, #21, #22** — the 2026-09-07
> triage covered **11**, so exactly one has dropped off since, and the remainder are the same
> class. That triage is written up in
> [#562](https://github.com/cairn-ehr/cairn-ehr/issues/562): nine were taint-through-an-argument
> (a secret passed INTO a function makes its non-secret return — a `PathBuf`, a `SocketAddr`, a
> count — look tainted), two are a CLI echoing back a patient UUID the operator supplied.
> `print_recovery_code` IS a real secret on stderr and is correct: all five call sites are
> interactive provisioning ceremonies and **no cron-run command reaches it** — re-check that if
> a future slice ever calls `resolve_or_adopt_unwrap_secret` from an unattended path.
> **Two human acts still owed, IN THIS ORDER:** dismiss the `cleartext-logging` alerts
> (per-alert verdicts in #527's comment), THEN make `CodeQL` a REQUIRED check. A
> permanently-red required check trains everyone to merge past it, which is how a genuine
> critical sat unread for a week.

> [!IMPORTANT]
> **⇒ #500 SPENT THREE DAYS *CLOSED ON GITHUB*, AND SIX OTHERS WITH IT (2026-09-04):** #101,
> #115, #434, #441, #468, #500, #534, all reopened. GitHub reads `close`/`fix`/`resolve`
> **adjacent** to a reference and never the sentence around it, so the sentences disclaiming the
> close performed it. Now guarded by `scripts/check_closing_keywords.py` +
> `.github/workflows/closing-keywords.yml` (which also prints what each merge WILL close);
> promotion to a required check is **#444**. **The commit convention `fix(#500):` is SAFE** —
> the parenthesis breaks the adjacency.
>
> **The reusable lesson, and the reason #500 hid for weeks:** *a deferral is only honest while
> its stated precondition holds, and nothing in the repo watches for one expiring.*
> `localstate.rs`'s header declared its seam truthfully — *"the federation-node tier has no
> clinical surface yet"* — and ADR-0052 made that false without reopening it, while ROADMAP kept
> recording slices A–D as ✓ done. **Before trusting any ✓, check whether the sentence that
> justified it is still true.** Slice 2b's grep found SEVEN more of this shape in FOUR crates
> where memory said one; **#511 then found two more inside `seal.rs` itself**, still describing
> the identity↔custody coupling ADR-0066 had deleted eleven days earlier. **Grep, do not
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
**#527's two Security-tab acts** — dismiss the triaged `cleartext-logging` alerts (**10** open as of 2026-09-10,
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

**Session date:** 2026-09-10, second session (**the DR restore's budget, measured — and the ruling 2d never wrote down.** Closes **#571** with **ADR-0068** (*provenance warns, never gates*; spec v0.69 → **v0.70**); **measures #512's §1.2 budget** — 100 003 events restore in **116.7 s against 600 s**, linear at 1.17 ms/event, 85 000 sealed bodies opening on the restored node. `M > N` still stands but the excess act **changed identity**. Opened **#572** (a restore cannot be scripted at all); **confirmed #552** with a number. No product-behaviour change, no migration, no SCHEMA bump.) · earlier that day: (**DR slice 2d — the record comes home.** `restore` reads the clinical plane back and a restored node's sealed body OPENS; closes **#554**, adds **ADR-0067** (spec v0.68 → v0.69) and **`db/052`** (SCHEMA 51 → 52); the pin `nothing_yet_restores_a_clinical_event_from_a_medium` **inverted, not deleted**; the quarantine pen moved into the database with two callers and gained custody; `ActorRegistryRow::recorded_at` lost its serde default. **Its §1.2 residual is discharged by the session above.**) · previous: 2026-09-07 (**the CAIRNB3 section-framing guard, #523** — a header vouches for its own length, so a corrupt length stops reading as an interrupted append; folded into the 2c branch, the last moment a pre-field format change is free. **The same session found 2c itself unmerged and un-PR'd**) · 2026-09-06 (**DR slice 2c** — the medium carries the clinical record, and nothing yet restored one; closed #522/#524, fixed #550 in-branch, opened #549/#551/#552) · 2026-09-04 (**the closing-keyword guard**: seven issues GitHub had closed that nobody closed, reopened + a CI guard) and, earlier that day, **#511** (**the custody newtypes**; opened #541) · 2026-09-02 (**DR slice 2b** — the transport seam and the paged pull; opened #531, #532, #534–#538) and, earlier, **#527** (the CodeQL backlog; opened #529, #530) · 2026-09-01/08-31 (**DR slice 2a** + its review wave) · 2026-08-30 (**#503**, the shared keystore crate) · 2026-08-24 (**DR slice 1**: #495 CLOSED). Earlier sessions: see *Recent sessions* below. · **Spec/ADRs:** **v0.70** ([ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md), refining 0067; and [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), which supersedes **ADR-0026 decision 2's implementation wording** only) · **`SCHEMA_GENERATION`:** **52** (`db/052`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

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

### 2026-09-10 (last) — the restore's budget, measured; and the ruling 2d never wrote down

**Closes #571 (ADR-0068, spec v0.70). Measures #512's time half. Opens #572. Confirms #552.** No
product-behaviour change, no migration, no SCHEMA bump. PR #573. What generalises past the slice:

- **⇒ A DESIGN SENTENCE WITH TWO READINGS AND NO TEST SURVIVED A MERGE, AND THAT IS THE LESSON.**
  2d's §5.2 said clinical segments *"inherit the same `Provenance` treatment the node plane already
  gets"*. The node plane's treatment is a **printed warning**; the same paragraph then called it a
  *"safety gate"* forcing `M = 3`. One reading was already built, the other existed nowhere, ADR-0067
  recorded **neither**, and the divergence was invisible because design test 18 was never written.
  The restatement leaves the accepted wording **struck through and standing** for exactly that reason.
- **⇒ THE ERRATA RULE SAID NO TO WHAT THE ISSUE ASKED FOR.** #571 asked for a paragraph appended to
  ADR-0067. `decisions/README.md` allows an erratum only where a passage is factually false **about
  the code**, placed below that passage — and ADR-0067 never mentions provenance, so there was nothing
  to sit under; decision-shaped content takes a new ADR. **Check the rule before honouring the ask.**
- **⇒ THE MEASUREMENT'S REAL RESULT IS THE SHAPE, NOT THE HEADLINE.** 116.7 s against 600 s is
  comfortable, but the useful fact is **linear at 1.17 ms/event with no bend**, which converts the
  budget from a pass/fail into a ceiling at ~510 000 events. A single dot could not have said that,
  which is why the rig runs a curve.
- **⇒ THE RIG REFUSES TO TIME AN INCOMPLETE RESTORE, AND THAT GUARD IS NOT PARANOIA.** A restore that
  applies nothing is **fast**. Driving the real binary produced that outcome on the first attempt (the
  piped recovery code, #572), and a rig that timed it would have written a flattering wrong number
  into a dated file that outlives the session.
- **⇒ A TEST CAUGHT A DUPLICATED COUNT BEFORE IT REACHED THE RESULTS FILE.** `Measurement` carried
  both a seeder-derived event count and the restore's own; they disagree, because the medium also
  holds the warm-up registration. The field was **deleted**, not reconciled — a second spelling of a
  count is a second thing that can be wrong, and this one would have been the published one.
- **⇒ THE SEEDER GOES THROUGH THE PRODUCTION ORCHESTRATORS, AND THE MIX IS LOAD-BEARING.** 85% of the
  corpus is born-sealed. A demographics-only medium carries **no `event_dek` rows**, so the per-record
  unwrap and re-wrap — the expensive half and ADR-0067's whole subject — would never run. Every
  medication assert is authored by an **enrolled human**, so the restore genuinely re-resolves authors
  through `actor_current` rather than leaving decision 1's registry re-entry barely exercised.
- **⇒ THE NODE'S OWN `device` ACTOR IS ENROLLED BY A CLI CEREMONY THE LIB DOES NOT EXPOSE.**
  `ensure_registration_actor` is private to `main.rs`, so the rig runs one real `patient-register`
  first rather than re-spelling it — the mirror-list defect class, avoided by paying one process start.
- **A `results/` directory beside the crate is the existing pattern** (`cairn-gui/cairn-gui-tauri/results/`):
  runbook, template, dated file. The runbook's own precedent holds — *a runbook nobody has executed is
  a runbook that does not work* — and executing this one is what found the pty problem.

### 2026-09-10 (earlier) — DR slice 2d: the record comes home (condensed)

**Closed #554.** ADR-0067, spec v0.69, `db/052` (SCHEMA 52), four crates. The per-slice narrative is
ROADMAP's; the traps that outlive it are in the trap list and ⇒ NEXT above.

**Filed, not fixed — all still open except #571:** **#567** (`verify-backup`'s OK is federation-only,
so a green verify says nothing about the plane a restore now applies; the comment that used to call
that scoping a safety property now calls it a gap) · **#568** (`do_requeue`'s custody-carrying arm —
the remedy every penned reason advertises — has zero tests; every test call site passes `None`) ·
**#569** (db/052's registry door silently discards a **content** conflict and leaves
`actor_event_id`/`seq` unvalidated) · **#570** (the restore CLI surface is untested, exit code
included) · **#571** (provenance does not gate the clinical plane and ADR-0067 does not say so —
**closed by the session above, ADR-0068**).

What still generalises:

- **⇒ THE HEADLINE TEST DECRYPTS A BODY, AND THAT IS THE POINT.** `apply_remote_event`'s `p_dek`
  feeds into `cairn_wrap_dek(p_dek, v_pub)` — **the door wraps what it is handed** — and both carriers
  hold keys already wrapped to this node. Piping either through would double-wrap every key in the
  clinic's record: rows present, well-formed, exactly the right length, counts agreeing,
  `verify-backup` green, and the defect surfacing months later when a clinician opens a chart. **A
  test that counted rows would have shipped it.**
- **⇒ THE DOOR'S RETURN IS NOT THE RECORD'S FATE**, the blind spot a second review round (five
  specialised reviewers) found two Criticals in. db/020 has two LENIENT arms that `RAISE WARNING` and
  admit — a DEK that does not open the body, an unregistered node unwrap key — skipping step 9
  entirely and **returning normally**. Right for a puller; for a restore it is the zero-patients
  outcome with a clean summary on top, signalled only by a Postgres `WARNING` nothing polls. Fixed by
  asking the DATABASE (`custody_landed`). Same round: every door error was penned as a verdict about
  the **bytes**, local faults included — split by `refusal_is_deliberate`.
- **⇒ THE GUARANTEE TEST CAUGHT ITS OWN MISSING STEP.** Written without the registry restore, it
  failed with *"signer … is not an enrolled, non-revoked actor"* — **the zero-patients outcome wearing
  a different costume**, arriving through the slice built to end it. Every clinical apply door resolves
  its author through `actor_current`.
- **⇒ FIVE OPERATOR MESSAGES LIED MID-DISASTER, ALL THE SAME SPECIES: a message that is confidently
  wrong is worse than none.** Records past `verified_through` dropped silently while the summary
  counted against the medium's full total (*"0 applied … of N"* at exit 0, in segment 0). A
  registry-only failure produced *"no key installed"* — **read the file, not the failure**. A missing
  registry and a missing key got the same remedy and one was false. The untrusted-records warning
  compared a **deduped** count against a **raw** one. And with no registry the restore warned, applied
  anyway into a pen nothing can drain, then printed the pen's standard promise.
- **⇒ THE COMPLETENESS WARNING WAS BUILT, THEN REVERTED — READ #549 BEFORE BUILDING IT AGAIN.**
  `restore` never asks whether the medium is complete, and the warning was still the wrong fix:
  **every duplicate apply burns a `seq`**, so holes are routine on any federating node and grow for
  the life of the medium. *"N event(s) are missing"* would have been confidently wrong most of the
  time — the same defect species the round was fixing.
- **⇒ `requeue` MUST NOT ABORT** — it is the command a restore's own output points at. Resolving
  custody exactly as the pull path does made it refuse on a divergence, and `load_or_create_key` would
  **silently mint a stray `node.key`**. Custody there is best-effort now (`load_existing_key` never
  creates).
- **A plaintext DEK must be BORROWED out of `Zeroizing`, never `to_vec()`d** — one unwiped heap copy
  per sealed record, on a machine mid-disaster.
- **A `cairn-node` test cannot pass a `serde_json::Value` as a parameter** (no `with-serde_json-1`)
  and **cannot use `$1::jsonb`** — tokio-postgres infers a parameter's type from its cast TARGET, so
  the payload travels as `$1::text::jsonb`. `Display` on a `tokio_postgres::Error` is the two words
  "db error"; `common::db_msg` is the shared accessor.
- **⇒ THE STALEST DOC WAS ON THE FUNCTION THE SLICE REWROTE**, and eight such sites across three files
  were corrected. A reader six months out trusts the function doc over the module header.
- **⇒ THE SESSION STARTED BY FINDING ITS OWN ⇒ NEXT HALF-DONE.** PR #565 was open as a DRAFT saying
  *"do not merge"* and had been **merged anyway**. `gh pr list --state all` before acting is what
  surfaced it — the rule paid for itself the first session after it was written.

### 2026-09-07 — the whole-branch review of PR #555, and the section-framing guard (condensed)

**Two Critical findings, both FALSE GREENS — the failure the whole slice exists to end, found
inside it.** (1) The CAIRNB3 **CONTINUATION** arm, the one that runs every night, had no identity
guard: `refuse_unsafe_legacy_succession` is gated on `superseded`, which only the LEGACY arm sets,
so a peer's medium holding clinical seqs 1..100 made this node resume at `seq > 100` — **our events
1..100 never captured**, `seq_gaps` seeing no hole, and `kit_verdict` reporting `Restorable` at exit
0. Closed by `refuse_foreign_continuation`, which refuses BEFORE any capture and stays silent when a
medium names nobody. (2) **A failed unwrap-key load still advanced export coverage**, so an export
carrying every wrapped DEK and **no key** reported `Restorable`; `ExportOutcome::Skipped`'s own doc
already named "a load failure" and the call site did not honour it. **The trap worth keeping: three
tests in `verify_backup_scope.rs` were green over exactly that kit**, because the fixture wrote no
`.unwrap` file on the reasoning that the degradation was "a warning, never a failure". A fixture
that means *a full kit* has to build one.

Rest of the round was comment rot with real consequences (`cairn-medium`'s headline invariant list
still called the #523 sentinel "deliberately NOT attempted here"; two operator messages still said
two verdicts were indistinguishable in the output that had just distinguished them), plus eight
issues: **#556** (`segment_commitment` does not bind `attestation`/`attester_key` — **free only
until a release ships a CAIRNB3 writer**), **#557**, **#558**, **#559**, **#560**, **#561**, **#562**
(the CodeQL triage) and **#563**.

**#523, the section-framing guard**, rode the 2c branch because it is a pre-field CAIRNB3 format
change and 2c is the writer that starts producing media — the last moment it is free. A section
header now vouches for its own length, so a corrupt length stops reading as an interrupted append.
**That session also found 2c itself unmerged and un-PR'd**, its remote holding four superseded
design commits a rebase had already reworded — which is the incident behind the every-session-ends-
in-a-draft-PR rule.


### 2026-09-06 — DR slice 2c: the medium carries the clinical record (condensed)

**Closed #522, #524 and — as titled — #500**, whose read half became #554 (closed by 2d, above).
Opened **#549**, **#551**, **#552**, **#553**; **#550** opened and fixed in-branch. `db/051`,
SCHEMA 51. The per-slice narrative is ROADMAP's; what generalises past it:

- **A BACKUP REPRODUCES THE STATE AT CAPTURE TIME, AND THAT IS NOT A LEAK** — the trap 7 rule
  below, now written into **ADR-0067 decision 2** in as many words, including that completing an
  erasure across backups is **rotation** whose interval IS the maximum time an erasure takes to
  complete across all copies (the clinic's policy call, not Cairn's).
- **CUSTODY TRAVELS ON BOTH CARRIERS, and the obvious one-authority design is wrong.** Putting the
  wrapped DEKs only in the `CAIRNL1` export — rewritten whole every run, so the shred filter applies
  retroactively — would make a shredded key unreachable *by construction*. The medium's copy is
  co-fresh with the events it unlocks; a restore reads a body if **either** still holds its key.
- **THE SHRED PREDICATE GETS ONE HOME, IN THE DATABASE (`db/051`).** *"A shredded body's key must
  not travel"* had two hand-written spellings in two crates and 2c would have been a third — the
  mirror-list defect class (#182, #404, #441) with a SAFETY predicate as the mirrored thing.
  ⚠️ The view is `security_invoker = true`, or it becomes a decoy path around db/037's custody
  REVOKE (the #430/#431 shape).
- **POSTGRES BURNS AN IDENTITY VALUE BEFORE CONFLICT ARBITRATION**, so permanent `seq` holes are
  routine and a watermark cursor loses events at one. The capture backfills gaps **newest-first**
  under a bounded probe budget — which is why 2d's reader must SORT by `source_seq` and why a
  fixture in capture order proves nothing.
- **A TORN TAIL MUST NOT REFUSE `restore`** — a ruling that REVERSED an earlier one in the same
  slice. Exact legacy parity is right for `verify-backup`, whose job IS to say "this is not a
  complete backup", and wrong for `restore`, where refusing converts a recoverable partial loss
  into a total one at the moment re-running the backup is usually impossible.


### 2026-09-04 — seven issues GitHub closed that nobody closed (condensed)

**Reopened #101, #115, #434, #441, #468, #500 and #534. Closes no defect; builds one guard. No ADR, spec
bump, migration or DB change.** Found while checking, before starting 2c, that the tracking state ⇒ NEXT
rests on was real. It was not: **#500 — the issue this whole file is organised around — had been closed
on GitHub since 2026-09-01**, one second after PR #526 merged. Full narrative in ROADMAP.

1. **⇒ THE SENTENCE WRITTEN TO PREVENT THE OVER-CLAIM IS WHAT PERFORMED IT.** GitHub matches a closing
   keyword **adjacent** to a reference and never reads the sentence: *"It does **not** fix #500"*, *"It
   does close #101 **item 1**"*, *"**Filed rather than fixed:** #534"* each closed what it disclaimed.
   The prose was accurate; the *machine* read three words of it. (Still biting: the 09-07 session's own
   PR body tripped the guard while DESCRIBING this defect — quote the shape, never the example.)
2. **⇒ A WRONGLY CLOSED ISSUE IS INVISIBLE, NOT WRONG-LOOKING.** Nothing surfaced any of the seven — not
   triage, not `/techdebt-loop`, not the ROADMAP prose still describing #441, #468 and #115's part 2 as
   open. **#115 sat closed for eight weeks.** The tell is timestamps: each closure is 1–3 s after a merge.
3. **⇒ THE GUARD HAD TO MIRROR GITHUB, NOT IMPROVE ON IT.** `fix(#500):` is **safe** — the parenthesis
   breaks the adjacency (proof: `fix(#288)`/`fix(#530)` sit on `main` with both issues open) — and a guard firing on nearly every commit here would be switched off within a
   week. `scripts/check_closing_keywords.py` reproduces GitHub's parser, then flags only a reference whose
   own clause denies it; it reads the PR **title** too (GitHub's merge commit carries it as its body —
   `(closes #38)` in PR #42's title closed #38 one second after merge). **Every false-positive shape it
   knows was found by running it over history — 216 PR bodies, 1650 commit messages — not by imagining
   inputs.** Plumbing: `scripts/collect_pr_text.sh`, with its own shell test, because a checker fed the
   wrong text is not a control.
4. **Residual: the check is not required.** Promoting it is admin-only — **#444**, under #527's ordering
   rule: only promote a check that is green on `main`.

### 2026-09-04 (earlier) — #511: the custody newtypes (condensed)

**Closed #511, opened #541, #543, #545.** Sequenced after 2b and before 2c because 2c/2d are where
key material moves again, so the newtypes had to exist before that code was written. Every key in
the custody plane was a bare `[u8; 32]`, so `destination.install(&unwrap_public(&secret))`
**compiled** — the #495 shape one layer up, on a surface no runtime check could catch.
`Secret32`/`PublicKey32` make the PUBLIC-for-secret mix-up a compile error **and nothing more**:
secret-vs-secret is NOT separated (trap 5 below). The inventory of `Secret32::from_bytes` call sites
lives **in code that fails**, per file and by count, because the review found the count asserted in
six places silently counting three different populations. `LocalState::unwrap_secret` is a
serialized `CAIRNL1` field, so `Secret32`'s hand-written `Serialize` reproduces ciborium's array-of-
uints encoding exactly and is **golden-pinned from the PRE-newtype build** — a round-trip cannot
catch a mirrored change (2a's 19/19 lesson). **#541:** `extensions/cairn_pgx`'s `pg_test` module had
not compiled for some time because **no CI job builds that cfg**.

### 2026-09-02 — DR slice 2b: the transport seam and the paged pull (condensed)

**Closed #101 item 1 only** (items 2–3 keep it open); opened **#531**, **#532**, **#534**–**#538**.
New `crates/cairn-wire` lifts the clinical-plane wire types and the transport seam out of
`cairn-sync`'s binary-only `main.rs` — the same wall that later put the quarantine pen in the
database (2d). `MediumTransport` is a CAIRNB3 medium answering as a peer. **Paging:** the serving
side fetches `limit + 1` and truncates, because `rows.len() == limit` cannot tell *"the log ends
here"* from *"there is one more we cut off"*, and a wrong `complete: true` at that boundary strands
every event above it forever. `do_pull` **commits its cursor AND its quarantine floor after EVERY
page** — that per-page durability, not the smaller frame, is #101 item 1's actual fix. `--page` is
refused above `MAX_PAGE_EVENTS = 8000`, since a much larger page puts the response back over the
64 MiB frame cap: the pathology paging was written to fix, reintroduced from the flag meant to tune
it.


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
- **⇒ DR slice 2d — #554, and it is the next build. DESIGN LANDED (2026-09-09), implementation not
  started;** draft PR **#565** on `feat/554-dr-slice-2d-restore-reads-clinical-plane`. 2a (the format,
  08-31), 2b (the transport seam + the paged pull, 09-02), **#511** (the custody newtypes, 09-04) and
  **2c** (the capture, 09-06) have all landed; **2d reads the clinical plane back off the medium** — the
  events, the carried `event_dek` rows and the carried actor registry — and is what closes #554.
  **Four things in the reviewed design contradict what earlier entries here lead a reader to expect —
  read these before trusting the 2b entry below or ROADMAP's 2e line:**
  1. **2d does NOT drive `cairn-sync`'s puller through `MediumTransport`.** The 2b entry below says the
     transport seam "is what lets 2d's restore drive `cairn-sync`'s OWN puller against a file"; the
     design takes the other route. `MediumTransport` is a *serving* abstraction (paging, label, logging
     latch) and `cairn-node` does not depend on `cairn-wire`. Instead the pure
     `within(verified_through) → sort by source_seq` derivation is **lifted into `cairn-medium`** (which
     `cairn-wire` and `cairn-node` both already depend on) and both consume it — one implementation of
     2a invariant 5, no new dependency edge. Follows 2c's Erratum E2 precedent.
  2. **The per-peer quarantine quota does not apply to a restore-originated pen**, and the pen becomes an
     in-DB door (`cairn_quarantine_event`, `db/052`) that `cairn-sync` and `cairn-node` share —
     `quarantine_event` lives in `cairn-sync`'s **binary-only** crate and `cairn-node` cannot call it.
     The quota's `Err` says *"the watermark freezes instead (delayed, never lost)"*, which needs a cursor
     and a re-serving peer; a restore has neither, so inheriting it would lose the record at exit.
  3. **The actor-registry door is set-shaped and resumable** (`restore_actor_registry(p_rows)`), not
     per-row. A door refusing "any row in `actor_event`" would refuse the re-run that the
     `finalize_identity`-moves-last ordering exists to make possible.
  4. **The ADR is this slice's, not 2e's** — ADR-0067, and it carries the ADR-0026-decision-2
     supersession plus the spec bump 2c deferred. See ⇒ NEXT. New
  from 2c: **#549** (a burned IDENTITY `seq` is indistinguishable from a lost clinical event; the
  probed-empty set wants a durable home and an operator surface), **#551** (the kit-restorability figure
  lives in a node-global file, not the kit — the same-path rotation case is still open) and **#552** (the
  nightly capture is O(whole medium), so the < 2 s budget is crossed at ~23 000 events). Still 2a's: **#525**
  (**#523** was 2a's too and is CLOSED by this branch — see the framing-guard entry below). New from #511:
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
  `CAIRN_TEST_PG`/`PG2`/`PG3` baked in (DBs `cairn_test`/`2`/`3`). **The CLUSTER is discovered, not
  assumed** — `scripts/pg-target.sh` finds a running PostgreSQL meeting the `db/001_envelope.sql` floor
  (>= 18), refuses one below it by NAME AND VERSION, and refuses to guess when two qualify; set `PGPORT`
  to name one. It used to default to `127.0.0.1:5532`, true of one machine in the world, and a bare
  `run-db-sql-tests.sh` reached whatever the libpq socket did — on this Mac a PG16 instance, which
  failed 22 migrations deep on `max(bytea)` and cost a wrong diagnosis (2026-09-07).
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
