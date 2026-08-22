# HANDOVER — Cairn

## ⇒ NEXT

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems. A, B,
the cross-cutting authority floor and the operator surface over all three are built; ⇒ C is next.** Read
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) and
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) before touching the rest; **do not
re-derive their decisions.**

- **BUILT.** **Part A** (Slice 65, ADR-0062) — graded append-only assertions over an event / a thread /
  a whole chart, the effective grade being the **max** over all three; computes and reports only.
  **Part B** (Slice 67, ADR-0063) — the precise `{class, severity}` is captured **pre-seal** and sealed
  with the body, while a **rung** chosen by the standing grade rides the envelope in the clear; emits a
  *signal*, enforces nothing. **The operator surface** (Slice 69) — `patient-sensitivity <chart>`, the
  one query that tells the whole truth, ADR-0064's §1.2 budget **MET** and pinned by a test (residual
  [#436](https://github.com/cairn-ehr/cairn-ehr/issues/436)).
- **The authority floor** (Slice 68, ADR-0064, spec v0.66) — a protection-removing claim takes effect
  only when a human this node can hold responsible stands behind it: ONE predicate
  (`cairn_claim_authority`, db/005) at exactly ONE site (the `NOT EXISTS` in
  `cairn_sensitivity_standing`, db/048), so display coarsening, safety-rung emission and part C's dial
  all inherit it structurally. It gives **#245** its first SQL counterpart — NOT its "mirror" (a word
  both `contributor.rs` and ADR-0064 retract), and NOT its display half. **Part C keys on this floor.**
- **⇒ Part C — sequester / custody narrowing** ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)):
  Slice 66 pinned custody to admission and Slice 68 closed the un-attested-strip hole a grade-keyed
  dial would otherwise have inherited. **What remains is the dial question, sharpened by
  ADR-0064 §8**: a custody dial *derived from* the effective grade is only as strong as its
  most-custodial holder — the grade is node-relative (ADR-0062 decision 9), so a well-custodied peer
  legitimately computes a *lower* grade and hands out the DEK on it. An **explicit custody act** (a
  signed `custody.narrowed`-shaped event) has no such property. **An input to #376, not a decision
  taken — do not treat it as settled.**
- **Part D — break-glass** ([#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)): audited key-*use*,
  partition-honest. Blocked on C.

**Two §5.9 leaks were closed 2026-08-16** (#412, #405), leaving two facts. **`REVOKE SELECT (column)`
is inert while a table-level grant stands**, so `cairn_agent` holds an explicit 23-column grant on
`event_log` omitting `safety` — and **adding a column to `event_log` now requires granting it in db/049
section 8** (fail-closed; `safety_read_grants.rs` names the missing one). And the correction that matters
most: **that grant is cost-raising, not a floor** (the column copies a *clear* field of the signed body,
and the runtime role keeps the table grant — **#425**, **#427**). **Never cite db/049 section 8 as a
confidentiality boundary**; ADR-0063 decision 2 binds. Whether a node should attempt one below the
envelope AT ALL is **[#432](https://github.com/cairn-ehr/cairn-ehr/issues/432)**.

Slice 65's own follow-ons still open: **#374** (thread resolution resolves only a thread's *current head*),
**#378** (the withdrawal rationale is clear text forever and replicates — the UI must warn at entry today),
**#379** (the grade in the twin) and **#436** (the mis-chart withdrawal when it arrives by replication).
**#374 and #379 each need a DECISION, not a patch** — #374 puts a body read on the safety-critical grade
path, and #379 must choose *which* grade an immutable artefact states, landing with #283 or the
demographic twin-match floor refuses a one-sided widening.

> [!NOTE]
> **CLOSED — the `arrayref` supply-chain incident (#445, 2026-08-20)**: a typosquat reached
> `cairn-event` via `blake3 → bao`, fixed upstream the same day, no code change. One residue open:
> `bao` is stale, so evaluating `bao-tree` is
> [#454](https://github.com/cairn-ehr/cairn-ehr/issues/454). The finding that outlived it — a
> gitignored `cairn_pgx` lockfile — is closed (#446); narrative in
> `crates/cairn-node/tests/cargo_lockfiles_tracked.rs`.

> [!IMPORTANT]
> **Two code traps that outlive their slices, repeated here because both look like tidy-ups.**
>
> 1. **`content_address IS NOT NULL` is the "did anything win" test — never `subject_kind <> 'none'`.**
>    The catch-all arm reports `'coarsened'`, and `none` is a legal open-vocabulary value that collided
>    with the sentinel (ADR-0062 E6).
> 2. **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0 withholds
>    *reject power* (safe); in the sensitivity and safety ladders it would withhold *protection* or mute a
>    warning. Aligning the three is the cleanup most likely to be attempted in good faith, and it reopens
>    a leak. Each `ELSE` carries a shouting comment for that reason — and each is pinned by a test.

**Three things still owed are HUMAN acts an agent cannot do:**

1. **The §1.2 time budget is a seeded figure, not a measured one.** Follow
   [`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md)
   (commands verified) into a dated copy of `TEMPLATE.md`. Only the *write* half is measured (median
   222 ms, hence **PARTIAL** in that result's title). Slice 63 owes BOTH halves for registration (≤ 5 s
   to find, ≤ 20 s to register); its write-cost half is
   [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360) — nothing is wired, and db/044's
   `gesture_kind` CHECK refuses a registration row until widened.
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`. The fixture
   chart deliberately carries a cross-patient line and an invisible group so the ADR-0060 warnings are
   exercised. Automating the DOM assertions is **#332** (needs a JS-toolchain decision: plain JS, no npm).
3. **Make two CI jobs REQUIRED status checks** ([#444](https://github.com/cairn-ehr/cairn-ehr/issues/444),
   admin-only) — "clippy + cargo test (cairn-gui)" and "cargo doc (API surface)". Both run on every PR;
   neither is in main's branch protection, so both can go red without blocking a merge. **Match the job
   names exactly** — a mismatch orphans the required check and blocks every PR silently. `CONTRIBUTING.md`
   carries the current state in a dated "jobs that run but do not yet block" table.

**If a measurement falls outside its budget, that is the finding — file an issue; never adjust the budget.**

**The other build candidates** (any can be picked up next; nothing blocks a choice):

1. **The registration/search UI slice** — the picker is the wrong-chart affordance paper has and the
   med-list window does not. **Constraint from Slice 63:** it must **open** a chart, never *retarget* an
   open window — retargeting re-creates the §5.8 item 4 / §5.11 windowing misfile possession semantics
   exist to prevent. Also wires the kept-but-unwired pane/routing/freshness machine.
2. **The drugref term→anchor lookup** — the §9 *advisory* tier, and what actually closes the
   **coded↔uncoded** duplicate case ADR-0059 decision 5 leaves open. Needs a design decision first: the
   cross-service connection model. The slice-6a/6b source guard keeps the trusted surface drugref-free
   and must stay passing. **Slice 67 gave it a second consumer:** `safety_class_map` is the empty seam
   drugref would populate.
3. **The node/actor plane's two divergences** — `db/007` fail-closes on an unmappable type (**#301**) and
   skips-and-advances a verifiable refusal where the clinical plane now pens (**#268**). **Neither is a
   symmetric fix**, and both are `loop:blocked`.

**Standing gate:** whole-project review cycles repeat periodically; there will be **no release for
clinical use before repeated cycles pass cleanly.** Last full pass 2026-07-15
([report](code_reviews/2026-07-15-whole-project-architecture-review.md), #187–#217), **fully closed**. A
runnable clinical surface exists that has never been through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
> session holds the main repo. **Never start it alongside one**: they contend on one cargo lock and one
> `test_serial_guard` advisory lock (a stray loop once stretched a session's suites ~3 → ~90 min). **A
> live IDE contends the same way**: rust-analyzer's `cargo check --workspace --all-targets` holds the
> shared `target/` lock, so a narrow `cargo test` blocks before it compiles, then times out. Fix is a
> scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the IDE — and keeping one warm is also what makes
> the full gate ~15 min. (The old "recreate cairn_test/2/3 after an `event_log` column add" note is
> OBSOLETE — #296 closed it structurally.)

---

**Session date:** 2026-08-22 (08-21's four passes, then three more: **#460**, the admit-and-flag repair of #370's door asymmetry, which had contradicted ADR-0063 · **#465**, the operator signal that repair owed · **#467/#469**, the two places a database failure said nothing) · **Spec/ADRs:** v0.66 (through **ADR-0064**; **no new ADR** in any of the three) · **`SCHEMA_GENERATION`:** 50 (`db/050`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — one line each; ROADMAP + the ADR log + git carry the detail:

- **Demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex /
  gender-identity · §4.3 address) · **the §5.2 advisory Python matcher** (in-DB veto floor, scoring core,
  veto-gated pipeline/blocking, B3 eval harness, compound keys, generator, weight-learning).
- **The §5.7 identity core C1–C5** (link · apply seam · auto-apply band · dispute · identify · repudiate
  + the alias pool; C5+ `reattribute` waits on a clinical-note surface) · **the §5.4 John-Doe subsystem**
  (slices A–D, photo/text evidence, the `enroll-human` ceremony; §5.12 push-alert open).
- **The §5.3/§5.8 search-before-create funnel** (ADR-0061) — the registration act, its db/045 floor and
  projection, the advisory db/046 search, `cairn-patient-search`, and the **precedence rule** (#345,
  db/005 step 8b; `patient.created` retired in db/047).
- **`clinical.medication` slices 1–6b** — assert/cease · bitemporal dose timeline · cross-thread
  reconciliation (ADR-0047) · attestation overlay (ADR-0049) · per-field dose correction (ADR-0050) ·
  inline `substance.coding` (ADR-0059); with the twin-check registry (ADR-0048) and the contributor-role
  floor (ADR-0051).
- **Born-sealed clinical bodies** (ADR-0052) · **per-write human authorship** (ADR-0053 — grading
  half-live until #245) · **the §5.9 stream COMPLETE through its read surface** (see ⇒ NEXT).
  **§5.9 enforces nothing beyond display/emission.**
- **The L3 reference UI** — `cairn-gui/`, a standalone workspace, one-way GUI → crates. The iced shell
  FAILED the accessibility bar (spike 0004, retired 08-03); today it is **`cairn-gui-tauri`**, a Tauri 2
  window on one patient's medication chart (plain JS, no npm). The pane/routing/freshness state machine
  survived the retirement, is tested, and is **not wired**.
- **The med-list node tier** (first clinical READ path + whole-list sign-off) · **generic reprojection**
  (ADR-0057) · the **ADR-0056 admit-uninterpreted floor** + the **residual refusal contract** (a
  deliberate refusal is penned verbatim, auto-released on repair; a frozen watermark fails loud).

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-08-22 (last) — the two places a database failure said nothing

**Closes [#467](https://github.com/cairn-ehr/cairn-ehr/issues/467) and
[#469](https://github.com/cairn-ehr/cairn-ehr/issues/469); `crates/cairn-node` +
`crates/cairn-sync` + `bet_a.py` — no schema change, no ADR, SCHEMA stays 50.** Both are #109's
species, and both were found by a real failure rather than by reading. Every item mutation-checked.

1. **⇒ `tokio_postgres::Error`'s `Display` IS THE STRING `"db error"`** — a bare match on kind that
   never chains to the source holding the message, the DETAIL and the SQLSTATE. Worse, `anyhow!("…:
   {e}")` *also discards the source*, so anyhow's chain printing has nothing left either: the wrapper
   meant to add context subtracted the diagnosis. That is the whole content of the CI line `loading
   031_medication: db error`. New pure `cairn-node::db_diagnosis` renders `message [SQLSTATE] —
   DETAIL`, **byte-identical to cairn-sync's `legible_db_error`** so an operator learns one format;
   all 9 `anyhow!` and 5 bare `?` sites in `db.rs` route through it, and the `connect` door **never
   echoes the connection string** (it can carry a password).
2. **A THREE-LINE LOOP BODY CANNOT BE TESTED**, and #467's acceptance criterion is a sentence about a
   *message*. The replay loop's body is now `db::load_migration`, a named door drivable with a
   deliberately failing body — which pins the criterion rather than restating it.
3. **⇒ A LOCAL DB WRITE FAILURE WAS LOGGED AS LINK DOWNTIME.** `do_pull`'s cursor commit is the ONLY
   propagation point between the apply loop and the metrics object, so a bare `?` there lost every
   number describing work that HAD happened — and `run`'s catch-all called it `"partition": true`,
   sending an operator to the WAN and charging the Bet A availability figure. New third class
   `CursorCommitError` → `"local_fault"`, via a pure `classify_pull_failure` whose default arm is
   `partition` **last**, so an unclaimed failure is the one where nothing was reached. `bet_a.py`
   counts and prints it — a key nothing counts is the silence the fix exists to end.
4. **⇒ A FAILED WRITE MUST NOT REPORT THE VALUES IT MEANT TO SET.** `cursor_seq` and `floor_active`
   are claims *about that write*; publishing them after it failed tells a monitor the cursor advanced
   when it did not — and after a mid-statement connection loss this node genuinely cannot know. Both
   go `null` (`mark_cursor_outcome_unknown`), `references_unlearnable`'s null-never-zero rule one
   field over, with the attempted seq named in the message instead.
5. **Test mechanic worth reusing:** to force a write failure in a SHARED test database, hold a `FOR
   UPDATE` row lock from a second connection under a short `lock_timeout`. A trigger or a `REVOKE`
   persists if the test panics and poisons every later suite; a row lock dies with its connection.

**Raised, not fixed: [#471](https://github.com/cairn-ehr/cairn-ehr/issues/471)** — `do_requeue` has
the same bare `?` sites *inside* its loop — **three**, not two (`query_opt` for the row, the release
`DELETE`, the `last_requeue_error` `UPDATE`) — so a failure on row 5 of 20 loses the whole report of
what the first four achieved. **ADR-0060 decision 2** (partial completion must be reported, never
implied) is the argument, and #469's fix is the worked example. Note the two halves are about
different errors: the `?`s propagate a raw `postgres::Error` (which renders `db error`), while the
`e` *stored* in `last_requeue_error` is an `ApplyError`, already legible. Both are real; only the
first is #467's species.

**PR #472 review round (2026-08-22).** The review of the fix above found one defect at its centre and
several around it; all are fixed on the same branch rather than deferred:

1. **`legible_db_error`'s fallback arm was #467 one kind over, and REGRESSED `db::connect`.**
   `tokio_postgres::Error`'s `Display` is a bare kind match for *every* kind, not just `Kind::Db`, and
   it never chains to `source()`. So `error connecting to server` was the whole of what the node said
   about a refused socket, an unresolvable host and a TLS timeout alike. Worse: `connect` was a bare
   `?` before the PR, which anyhow preserved, so `main`'s `Termination` printed
   `Caused by: Connection refused (os error 61)` — and wrapping it in `anyhow!("…: {}")` deleted that.
   The arm now walks `source()`. **`hint()` was also dropped at all 14 sites** (`DbError`'s own
   `Display` prints message + DETAIL + **HINT**), which on a `42883` throws away PostgreSQL's own
   remedy; the composer now carries it, labelled.
2. **The test covering that arm could not fail** — it asserted only `!= "db error"` and non-empty,
   both true of the broken output. Mutation-checked red before the fix.
3. **#469's misdiagnosis survived two statements above the fix.** `do_pull`'s first two statements
   (the `sync_state` upsert and read) are bare `?` on a `postgres::Error`, *before* any network I/O,
   and fell to `classify_pull_failure`'s default arm as `partition`. Since `do_pull` reaches its peer
   over a raw `TcpStream`, **any** postgres error escaping it is by construction local — so there is
   now an explicit arm for it. This mattered more than the case it generalises: `cmd_run` builds its
   client ONCE outside the loop, so after the database goes away *every* later cycle was logged as
   link downtime for the life of the process.
4. **The operator line contradicted the metric.** It said "the cursor did not advance" while
   `mark_cursor_outcome_unknown` nulled `cursor_seq` precisely because the statement may have
   committed before the failure. Principle 4 applied to the field and dropped in the sentence a human
   reads. Hedged now; the self-healing claim, which holds either way, survives.
5. **A failed commit swallowed a co-occurring integrity condition.** The early return sat above
   `cycle_is_loud`, so a cycle that quarantined events *and* failed its commit lost `integrity` and
   all of `loud_pull_message`'s remedies — while the local-fault line said the events "re-apply
   idempotently", which is the wrong remedy for a quarantined event. The classes are now a SET.
6. Smaller: `CursorCommitError`'s doc claimed it satisfies `cycle_is_loud` (it does not — the
   canonical case is `(0,0,false,false)`); "four operator actions" over five SQLSTATEs; "four
   characters" for the eight-character `db error`; a 0-row cursor commit reported `Ok`; `elapsed_ms`
   and `references_unlearnable` were newly nullable and asserted nowhere; the connection-string leak
   assertion was vacuous on a rig with no password.

**New guard:** `crates/cairn-node/tests/db_errors_stay_legible.rs` — a source scan asserting no `{e}`
interpolation survives in `db.rs`. The point is not the fourteen sites that exist but the fifteenth,
written by someone who has never read #467. Scoped to `db.rs` today because `safety.rs` (#473) and
`sync.rs` (#474) would fail it; widen it when they are fixed.

**Raised, not fixed — four, from the same review.** [#473](https://github.com/cairn-ehr/cairn-ehr/issues/473):
`safety.rs`'s advisory-lookup failure line says `db error` — on a CLINICAL surface, and its own doc
says the line exists so a degraded safety projection is distinguishable from a correctly empty one.
Reached on a real medication write path; arguably a better candidate for #467's fix than the schema
loader was. [#474](https://github.com/cairn-ehr/cairn-ehr/issues/474): `cairn-node`'s pull loop — the
P0001 deny-all reason, the FREEZE reason, and a `PARTITION` line asserted over an unread error (#469's
defect in the other crate); plus `db.rs`'s `let _ = connection.await;`, which discards every
mid-session connection death and is *why* a later client can only say "connection closed".
[#475](https://github.com/cairn-ehr/cairn-ehr/issues/475): `cairn-sync init`'s migration loop has
`name` in scope and unused in the error — #467's acceptance criterion, unmet in the operator's first
command. [#476](https://github.com/cairn-ehr/cairn-ehr/issues/476): ~120 further comments still say
advisory locks are "cluster-wide"; the ones stating the wrong MECHANISM are fixed here.

### 2026-08-22 (later) — the signal admit-and-flag owed

**Closes [#465](https://github.com/cairn-ehr/cairn-ehr/issues/465); `crates/cairn-sync` only.** After
#460 a peer event with an unlearnable reference produced a successful cycle, exit 0 and a log line
byte-identical to a healthy pull. It now gets a `references_unlearnable` metric AND its own stderr line
on **both** admit paths (`pull` and `requeue` share the db/020 door). ROADMAP carries the pass in full;
four things a next session can still break:

1. **⇒ A FLAG CAN BE BORN ON A RE-APPLY, AND v1 REPORTED `0` FOR IT.** db/020 calls
   `cairn_learn_attachment_refs_lenient` **unconditionally** (after its `ON CONFLICT DO NOTHING`, no
   `v_rows` guard), so a first-ever flag row can land for an event already held — a node upgrading onto
   db/050 flags its whole pre-#460 back-catalogue and would have said `0`. Keyed on **two** things now:
   the addresses the door admitted (another peer's defect is never charged here) **and a `flag_id`
   watermark** taken before the run (a re-delivery writes no row and stays silent). Drop either clause
   and five tests go red.
2. **`cycle_is_loud`'s two exclusions share ONE test:** the event set is COMPLETE and the loss is
   declared elsewhere. Anything else that leaves this node not holding an offered event is loud.
3. **⇒ PEER TEXT IS NOT DISPLAY TEXT — TWICE.** The ledger `reason` carries a bounded 8-char prefix of
   the peer's value (the DETAIL beside it, not the prefix, tells a truncated digest from a
   wrongly-encoded one), escaped `{:?}`. The review then found the LARGER channel on the very line cited
   as its precedent: `custody_withheld` is **unbounded prose from an unadmitted peer**, printed raw —
   enough to forge a `0 attachment reference(s)` all-clear on the alarm #465 had just installed.
4. **A failed read reports `null`, never `0`** — zero is a claim, and after a failed read it would mute
   a monitor exactly when this node stopped being able to see. `cairn_attachment_flag_health()` finally
   has a caller (`cairn-sync attachment-flags`), naming an example event per group **and** a total.

**Still open from #460's review:** **#463** (resolution path — a DECISION, overlay vs delete) · **#464**
(unbounded per-rendition subtransactions) · **#458** (non-object attachment element — a loud UI, NOT a
floor rule). **Raised by that pass and still open:**
**[#468](https://github.com/cairn-ehr/cairn-ehr/issues/468)** (**this alert fires ONCE EVER** while
`custody_withheld`, its stated precedent, re-fires every cycle — and an unlearnable reference is
unrepairable in place, so the steady state carries no standing signal) ·
**[#470](https://github.com/cairn-ehr/cairn-ehr/issues/470)** (the per-cycle ledger read is
owner-privileged — cairn-sync cannot carry it onto the unprivileged runtime role). **#467/#469 are
CLOSED** (entry below), opening **[#471](https://github.com/cairn-ehr/cairn-ehr/issues/471)**.

### 2026-08-22 — admit and flag: the rule was already written, under another field's name

**Closes [#460](https://github.com/cairn-ehr/cairn-ehr/issues/460); `db/050`, SCHEMA 49 → 50; NO new
ADR — and that is the finding. #461 raised one and was CLOSED unbuilt (maintainer): the rule stays
findable only under `safety`'s title, an accepted cost mitigated by db/027's and db/050's headers.
ROADMAP carries this pass in full; these are the parts a next session can still break.**

1. **⇒ #370's FIX CONTRADICTED AN ADR WRITTEN EIGHT DAYS EARLIER, AND NOBODY LOOKED.** ADR-0063
   decides this exact shape for the §5.9 `safety` field, in a table (*malformed field: local door
   REFUSE, remote door ADMIT*), and its rejected-alternatives argument never mentions `safety`:
   refusing a field at apply drops the clinical act it rode on, and **forks the event set between
   honest peers** — the #342 trap, hit four times before #370 made it five.
2. **THE CATEGORY TEST, which is the part worth memorising.** A sensitivity assertion **IS** an
   event — refusing a malformed one drops that assertion and nothing else. `safety`, `clock_grade`
   and an attachment rendition reference are **FIELDS ON** a clinical event. **Blast radius, not
   category** — ADR-0063's own deciding argument.
3. **⇒ THE RULE HAS THREE IMPLEMENTATIONS AND NO NAME** (db/040's `clock_grade` · ADR-0063's
   `safety` · db/050), *mint-strict, arrive-permissive*, **which is why it keeps breaking**.
4. **The one thing not to "align":** `submit_event` calls the strict learner (db/027),
   `apply_remote_event` the lenient one (db/050). They share their accessors **and** their traversal
   (`cairn_by_reference_renditions`, pinned by a `pg_proc` read in `db/tests/050` §9) and differ only
   in `EXCEPTION WHEN raise_exception` → record instead of raise. **`WHEN OTHERS` there would be a
   disaster** — a disk error written into the ledger as "the peer sent garbage", and cairn-sync
   robbed of the SQLSTATE it needs to retry.
5. **⇒ THE REVIEW FOUND PROSE ASSERTING SAFETY PROPERTIES THE CODE DID NOT IMPLEMENT — over a fully
   green branch.** And a **REGRESSION**: admitting a non-array `attachments` meant *storing* it, and
   `read_photo_refs` (`patient/search.rs`) walks it with `jsonb_array_elements` → **22023** on the
   §5.3/§5.8 candidate list, the wrong-chart-prevention surface. **The refusal had not been removed,
   only relocated** — out of a door that pens and names it, into a read path with no handling.

### 2026-08-21 — the freeze that hid, the flake that lied, six trap-clearing fixes, three silent gates

**Closes #370, #457, #449–#453, #386, #381/#382/#385/#439, #446/#442/#443; opened #458.**

- **⇒ PROBE THE FAMILY BEFORE FIXING THE MEMBER.** #370 named one field; measured, that one function
  had **nine** freeze paths across four SQLSTATE classes **and four SILENT paths that wrote something
  wrong**. The rule the fix follows: **refuse what already FAILED plus what was silently WRONG;
  accept everything that already worked** — every refusal added at a remote door is a new way for a
  peer's clinical event to be penned.
- **⚠️ A DATABASE-FREE `cargo test` FAILS UNLESS YOU DECLARE IT: `export CAIRN_ALLOW_DB_SKIP=1`**
  (#450; #451 gives the matcher the same variable). The polarity subtlety: an **opt-out** must read
  an unrecognised value as *NOT permission*, or `CAIRN_ALLOW_DB_SKIP=please` quietly restores
  fail-open. Only `1`/`true`/`yes`/`on` opt out.
- **#457 — the harness polled a PORT and never the CHILD**, so three unrelated causes produced one
  message blaming startup latency. It now watches both and captures stderr **to a file, never a pipe**
  (an unread pipe blocks the child). **Cause named, not fixed:** a macOS `_dyld_start` loader stall.
- **Check a claim against the pinned source before writing it down** — a load-bearing comment saying
  `std`'s `TcpListener` does not set `SO_REUSEADDR` was false and had steered two rounds of fixes.
- **Three mechanics.** (a) PostgreSQL checks a function called inside a VIEW against the **INVOKING**
  user — the INNER call too. (b) **Ask the authority:** `cargo locate-project` found `packaging/crates`
  was in no workspace. (c) `git check-ignore` needs `--no-index` and has **THREE** exit codes
  (0/1/**128**). Every repo cargo invocation in `rust.yml` now passes `--locked`.
- **A mutation that does not change the property tests nothing.** **Residual: #447**, **#327.**
### Older passes (Slices 61–69, 2026-08-02 → 08-20) — the lessons still worth holding

ROADMAP carries every slice in full. These are the ones a next session can still break.

1. **⇒ A guard defined over the list it guards is not a guard.** `assert_eq!(SubjectKind::ALL.len(), 3)`
   over an `[SubjectKind; 3]` compared a compile-time constant to its own literal and could not fail.
   **Ask what independent source a guard checks against; if the answer is "itself", it is documentation
   wearing a test's clothes.** Constructively: **where a family HAS an authoritative list, read the
   list** — and when reading a catalogue, **`proacl`'s NULL ACL is the PERMISSIVE case**.
2. **⇒ AN OPTIMISATION REMOVED A LOAD-BEARING REDUNDANCY, AND ITS COMMENT ASSERTED THE OPPOSITE.**
   #385's draft said widening §10b's thread-free list could only over-protect; measured, it is the
   reverse — §11's bound is gated on the NEGATION of the same predicate, so a type added to the list
   is EXCLUDED from the bound *and* stops resolving, and a standing `sequestered` grade reads back
   `('routine','none')`. **Before #385 the identical edit was harmless.** Carry: when an optimisation
   makes two paths share a predicate, ask what redundancy that destroyed; and **a wrong safety
   argument is worse than none.**
3. **NAME, NEVER COUNT** — a count cannot separate **custody-blind** from **genuinely empty**, the one
   question `patient-sensitivity <chart>` exists to answer. Related: **a union view whose arms mean
   opposite things must never get one summary sentence**; **the report declares what it cannot
   contain**, asserted over an **empty** list; and **peer text is not display text**.
4. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`** —
   ADR-0064's KNOWN GAP. `cairn_sensitivity_standing` is patient-scoped on both sides (load-bearing —
   else chart B strips chart A), so a mis-charted withdrawal's target IS absent here and a naive
   membership test reports it **effective**: a precise untruth in the reassuring direction on a
   confidentiality surface. **#436** is the residual, and it is visibility, not a door.
5. **A pinned `search_path` must deny the temp schema the FIRST look.** `SET search_path = public` does
   not exclude it, so with a decoy `event_log` in place both write doors **returned SUCCESS while the
   owner-privileged INSERT landed in the caller's temp table** — live data loss, as a role with no write
   privilege on `event_log` at all. Open: **#430** (~100 unpinned invoker-rights functions), **#431**.
6. **A parameter name is not a security property.** `classify_authorship_confidence(&body.contributors,
   &body.signer_key_id, None)` compiled, read naturally, and graded a forgery `Attested`; both key
   arguments are now a `VerifiedKid` newtype (mint-site allowlist unpinned: **#428**). **`attester_key`
   alone is NOT proof** — db/020's deferred arm stores a peer's token unverified.
7. **Slice 68:** the authority floor **gates effect, never admission**, and only in the withholding
   direction, so no fork (the **#342** trap); and **computing the verdict at read cuts both ways** —
   revoking an actor silently re-raises grades they lawfully declassified (**#409**), while the Rust↔SQL
   mapping diverges on two shapes (**#408**, root cause **#413**). **Flag what cannot self-heal, view
   what can.** PR #410's review: **7 of 11 production mutations survived a green suite.** Mechanic:
   **`EXCEPTION WHEN OTHERS` does not catch a statement timeout** (`OTHERS` excludes `query_canceled`,
   57014). Open: #413–#420, #422; **#415** measures the SIGNER, so **expect noise**.
8. **Slice 67 — the seal boundary is the coarsening boundary:** precise `{class, severity}` travels
   sealed, a grade-chosen **rung** rides the envelope in the clear, so *coarsen-but-survive* after a
   crypto-shred is structural. Emission coarsening binds a peer's raw-SQL client; **read coarsening is
   a rendering choice, not a floor**. `safety_class_map` ships **EMPTY** — drugref's seam. Open:
   **#406**, **#407**, #394–#402.
9. **Slice 66 — withhold the key, never the bytes** (the rule #460 applies one level down): the
   unwrap-cert kid is pinned to `trust_peer` (db/007), because refusing the bytes would fork the event
   set; repair is TWO steps (`pull --full`, then `cairn_reproject()`).
10. **Slices 61–63 — the seam and the surface.** An attestation **NAMES** the displayed candidates, it
    does not count them (§1.2 write-cost half: **#360**); **a displayed row is a GROUP, an attestation
    is a THREAD** (ADR-0047/0049 — nearly every defect lived on that seam); **a unit-tested safety
    control can still be defeated by the surface that calls it** — **test the path the product actually
    calls**; and **a compensating control outside CI is not a control** (**#444**).

> [!IMPORTANT]
> **The loud failure belongs in the UI, not the floor** (maintainer decision 2026-08-22, from #458).
> *If an attachment — or anything like it — is defective or unacceptable for any reason, the **user
> interface** is where it must fail loud, with immediate feedback, and **without blast radius for the
> rest of the clinical event**.* Three consequences for the attachment UI: **validate the rendition
> reference before submit** (the submit door refuses the whole event, correct only as a backstop that
> never fires); **fail at the attachment, not at the save**, while the clinician is still looking at it
> — a photo that will not stick is obvious when you try to stick it, and does not invalidate what is
> already on the page; and **no confirmation dialog** (principle 3). The same decision refused a
> mandatory `descriptor` as a floor rule: **principle 4 forbids a required field satisfiable only by
> fabrication** — a rushed clinician types `x`, and an honest absence becomes a precise untruth.

> [!IMPORTANT]
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md):
> *partial validity — a defect on one line never invalidates another.*** Read before any
> composite-clinical-object work: **the system may fail to record an order, but it may never cancel
> one.** Hold decision 2 (partial completion must be reported, never implied) and decision 7 (check the
> transaction boundaries).

**Five repo conventions these runs learned the hard way:**
- **A pinned COUNT lives beside the thing it counts, and a new member must be added to it.** A new
  `cairn_decode_hex_or_raise` call site fails `hex_decode_helper.rs`'s
  `every_hex_door_still_calls_the_helper`, which asserts an exact per-file call-site list; the twin and
  projection registries carry the same shape. The count failing is the guard WORKING — fix the list, and
  say in a comment why the new site is there.
- **Guard before connect** — take `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
- **UUIDs bind as text.** `cairn-node` does not enable tokio-postgres's `with-uuid-1`, so a `Uuid`
  parameter has no `ToSql`. Bind `&uuid.to_string()` and cast in SQL: `$1::text::uuid`.
- **A second human actor needs a distinguishing determinant.** `actor_id` content-addresses the *pinned
  determinant set*, so enrolling two clinicians as `{"role":"clinician"}` collides into one actor and is
  refused (P0001, ADR-0044/[#152](https://github.com/cairn-ehr/cairn-ehr/issues/152)). Use
  `enroll_human_with_role`. The floor working as designed.
- **`cargo test --lib` does not catch an import used only under `cfg(test)`** — it compiles the lib WITH
  `cfg(test)`. The integration build fails it under `-D warnings`. Use `--all-targets` (Slice 69).

**Earlier sessions — condensed.** ROADMAP carries the per-slice detail (Slices 13–60, both
tech-debt-loop "Interlude" entries, every still-open issue). Two lessons from Slice 60: **a refusal that
persists nothing is a refusal you cannot audit**, and **when a call site cannot make a distinction, check
whether an intermediate layer threw it away** (`apply_signed` flattened `postgres::Error` to `String`,
discarding the SQLSTATE separating a deliberate refusal from a transient fault). Arc 2026-06-25 → 08-01:
demographics + matcher · identity/John-Doe/medication · five-priority review → ADR-0051–0058 · ADR-0059
+ medication 6a/6b · the ADR-0056 admit-uninterpreted floor · floor determinism (#75) · loop launch.

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in `scratch/ui-sketches/`; source
screenshots git-ignored under `docs/untracked_for_brainstorming/` — real photos, **never commit or
publish**. Headline: easyGP's six editing-area invariants ≅ Cairn's event envelope near line-for-line.
**Scope:** the easyGP co-author may lead GP-facing GUI, HH designs ED & ward; the role-manifest layer
is the seam (ADR-0021). The open half is under "Blocked on external access" above.

**Status of this file:** disposable scaffolding, **not** a source of truth; canonical docs win.
Regenerate each session, **under 500 lines** (#368) — *why* in the ADRs, *what* in the spec.

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not.
- **`docs/spikes/`** — 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor
  — C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓); 0004 (iced UI — FAIL on a11y →
  Tauri 2). **`docs/case-studies/0001`**: 16 Australian GP-software failure modes, all absorbed, **0 new
  architecture**. **`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance.
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
  `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace).
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS
  pinned to the trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate,
  genesis-stable `node_id`. **Every honest gap declared at build time is CLOSED** — only optional escrow
  *rungs* (Shamir/QR/TPM) remain; the `localstate` seams are where the clinical tier plugs
  DEKs/drafts/config.
- **Dual-identifier discipline** (ADR-0031) — the canonical plane (UUIDv7 + multihash) is the *only*
  identifier on the wire/in signed bodies; the projection plane may intern node-local `bigint`
  surrogates (`db/008` + the leakage guard).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`) and self-serialize
  via a Postgres advisory lock (`db::test_serial_guard`). **Not "cluster-wide" — advisory locks are
  scoped PER DATABASE** (this file said cluster-wide until 2026-08-22; #467 caught it), which is why
  every caller must take the guard against `CAIRN_TEST_PG` specifically, whatever database its own
  work then uses. Strings and runner under Open threads → Test env.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels and drives `/techdebt-next` one
  fresh headless session per issue (`tail -f ~/.cairn-loop/run.log`). Auto-merge **ENABLED**; **works
  unattended** (12 PRs); **stopped** by maintainer decision — see ⇒ NEXT. Live gaps: **#326**,
  **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts C/D** (#232) — see ⇒ NEXT. Related: **#235** (shred authorization policy hooks),
  **#236** (FTS/RAG must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059, 2026-07-28). **Next candidates:** the
  **drugref term→anchor lookup** (⇒ NEXT item 2); fuzzy/automatic reconciliation + a Tier-A drug
  dictionary; structured sig/frequency (lands with prescriptions); correcting a dose event's *effective
  date* on the statement-level `started`. **Cross-cutting debt: #185** (cross-thread correction
  *suppression* — needs a PK/design decision). Spine: `db/031`–`db/033`, `db/041`, `db/042` +
  `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine: `db/010`–`db/030` +
  `cairn-event::demographics`). **Next (B3 measurement-driven):** a large hand-crafted gold set to re-run
  the learner for authoritative magnitudes; locale comparator packs; the hub-tier duplicate sweep;
  proposal retraction. **Next identity:** C5+ `reattribute` (**waits on a clinical-note surface**); the
  §5.12 push-alert. Deferred: **#168**, **#287**; the rest are in ROADMAP.
- **Test env:** DB-gated + matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532
  user=hherb dbname=cairn_test"` (PG18+cairn_pgx), plus `CAIRN_TEST_PG2`/`PG3` (`cairn_test2`/`3`, same
  cluster) for the multi-node convergence suites — without them those **self-skip and cargo counts them
  as passed** (CI sets all three, #199). **Since #450 a run without them FAILS unless it declares
  `CAIRN_ALLOW_DB_SKIP=1`** — Rust and Python both; only `1`/`true`/`yes`/`on` opts out. Matcher
  integration: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest`; the pure suite is
  dependency-free (`uv run pytest`) — uv, never venv/pip. The `db/tests/*.sql` **mirrors run only via
  `scripts/run-db-sql-tests.sh`**, which drops, recreates and marks a throwaway `cairn_sqltest`: since
  #169 each mirror refuses a database lacking the `cairn_scratch_database` marker, because the mirrors are
  destructive. **`scripts/run-db-gated-tests.sh` runs the mirrors *and* the full workspace with all three
  strings baked in — the one command for the DB slice of the local gate** (a warm `CARGO_TARGET_DIR`
  makes it ~15 min, not the 2 h a cold one costs). Local gap:
  [#314](https://github.com/cairn-ehr/cairn-ehr/issues/314) (it does not run the matcher DB-gated pytest
  suite; CI does). **`clinical_pull` used to flake under a full-workspace run.** #457 fixed the
  DIAGNOSTIC, not the cause: a stall now says whether the child died or hung and names a live pid to
  `sample`. **The cause is still unnamed**; serialising (`--test-threads=2`) remains the workaround.
- **Clinical case-mining** — historically the highest-signal generative mode; the primitives have
  absorbed every case so far without new architecture. Bring a real ED/hospital failure mode; record in
  [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation** (order/recall/referral with no closing ack) — a named
  projection surfaced by salience, not a modal; **③ impossible-vs-uncertain** for the in-DB floor.
- **Landing-page polish** — non-developer page for the generated site (`web/`; draft plans under
  `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)):
  **PASS twice** (clean 2026-07-07 re-run — B1 p95 **3.99 ms @ 2,004,000 events**, 13× under budget; B4
  confirms ADR-0015's BLAKE3 default). **Remaining:** fold the un-caveated B4 number into ADR-0015 to
  drop "provisional" from the blob-digest line, and
  [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (reproject bench on the Pi rig).
- **easyGP session** — port [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s
  deferred items with live schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon. Pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **easyGP GUI-mining continuation** — more consult-screen/module screenshots incoming from the co-author;
  they should answer most of the remaining §4.4 open questions in
  `scratch/ui-sketches/easygp-consult-screen-inventory.md` and open the **results/inbox design session**
  (three-zone vs two-pane is parked there — don't improvise it).
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection
  per slice. The §8.2 availability + windowing/resume work shipped.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** stewarding legal entity & jurisdiction — deferred
  until momentum/funding geography is clearer; formal trademark registration — principle recorded
  (stewardship doc), legal instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session** — the working
  conventions, the twelve founding principles (the first four being the lens for every design choice),
  and the §9 defect-blast-radius language rule. Not restated here; canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr`
  org; `cairn-ehr.org`+`.com`; PyPI/crates.io/npm `@cairn-ehr` placeholders).
