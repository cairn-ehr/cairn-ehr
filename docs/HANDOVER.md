# HANDOVER — Cairn

## ⇒ NEXT

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems. A, B,
the cross-cutting authority floor and the operator surface over all three are built; ⇒ C is next.** Read
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) and
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) before touching the rest; **do not
re-derive their decisions.**

- **Part A** (Slice 65, ADR-0062) — graded append-only assertions over an event / a thread / a whole
  chart; the effective grade is the **max** over all three. Computes and reports only.
- **Part B** (Slice 67, ADR-0063) — the precise `{class, severity}` is captured **pre-seal** and sealed
  with the body; a **rung** chosen by the standing grade rides the envelope in the clear. Emits a
  *signal*; enforces nothing.
- **The authority floor** (Slice 68, ADR-0064, spec v0.66) — a protection-removing claim takes effect
  only when a human this node can hold responsible stands behind it: one predicate
  (`cairn_claim_authority`, db/005) at exactly one site (the `NOT EXISTS` in
  `cairn_sensitivity_standing`, db/048), so display coarsening, safety-rung emission and part C's dial
  all inherit it structurally. It gives **#245** its first SQL counterpart — NOT its "mirror" (a word
  both `contributor.rs` and ADR-0064 explicitly retract), and NOT its display half, which stays open.
  **This is the floor part C keys on** — read it before touching sequester.
- **The operator surface** (Slice 69, 2026-08-18; closes #388, #383, #421) — `patient-sensitivity
  <chart>`, the one query that tells the whole truth: the withdrawal worklist (each arm stating whether
  it took effect), deferred `sensitivity.%` events, the standing assertions a custody-thin node cannot
  anchor, safety overclaims, and the **measured** count of sealed medication events held without custody.
  **ADR-0064's §1.2 budget is MET** (errata E1/E2), pinned by a test. Its review follow-ons #434/#435 are
  CLOSED, which opened [#436](https://github.com/cairn-ehr/cairn-ehr/issues/436).
- **⇒ Part C — sequester / custody narrowing** ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)):
  Slice 66 (#231) pinned custody to admission and Slice 68 closed the un-attested-strip hole a
  grade-keyed dial would otherwise have inherited. **What remains is the dial question, sharpened by
  ADR-0064 §8**: a custody dial *derived from* the effective grade is only as strong as its
  most-custodial holder — the grade is node-relative (ADR-0062 decision 9), so a well-custodied peer
  legitimately computes a *lower* grade and hands out the DEK on it, and no amount of authority hardening
  changes that. An **explicit custody act** (a signed `custody.narrowed`-shaped event, not a value
  derived from the sensitivity stream) has no such property. **This is an input to #376, not a decision
  taken — do not treat it as settled.**
- **Part D — break-glass** ([#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)): audited key-*use*,
  partition-honest. Blocked on C.

**Two §5.9 leaks were closed 2026-08-16** (#412, #405). Carry two facts. **`REVOKE SELECT (column)` is
inert while a table-level grant stands**, so `cairn_agent` holds an explicit 23-column grant on
`event_log` omitting `safety`, and **adding a column to `event_log` now requires granting it in db/049
section 8** (fail-closed; `safety_read_grants.rs` names the missing column). And — the correction that
matters most — **that grant is cost-raising, not a floor**: the column copies a *clear* field of the
signed body, so `cairn_body(signed_bytes) -> 'safety'` still returns it uncoarsened, and the runtime role
is a `cairn_node` member which keeps the table grant
([#425](https://github.com/cairn-ehr/cairn-ehr/issues/425),
[#427](https://github.com/cairn-ehr/cairn-ehr/issues/427)). **Never cite db/049 section 8 as a
confidentiality boundary**; ADR-0063 decision 2 (emission-time coarsening) binds. Whether a node should
attempt a confidentiality boundary below the envelope AT ALL is
**[#432](https://github.com/cairn-ehr/cairn-ehr/issues/432)**.

Slice 65's own follow-ons still open: **#374** (thread resolution resolves only a thread's *current head*
— erratum E4 narrows it), **#378** (the withdrawal rationale is clear text forever and replicates — the UI
must warn at entry today), **#379** (the grade in the twin) and **#436** (the mis-chart withdrawal, when it
arrives by replication). **#386 is half-closed** — db/049's subset test *drives* it; db/048's still does
not. Closed: **#383/#388** (2026-08-18) · **#434/#435/#387** (08-19) · **#381/#382/#385/#439** (08-20).

> [!WARNING]
> **SUPPLY CHAIN, LIVE AS OF 2026-08-20 — read before running any `cargo update`.**
> **[#445](https://github.com/cairn-ehr/cairn-ehr/issues/445): do NOT run `cargo update -p arrayref`,
> and do NOT set `[advisories] yanked = "warn"` to get `cargo-deny` green.**
>
> `cargo-deny` is red **on every branch, `main` included** (`Cargo.lock` is byte-identical across them,
> so this is not any PR's doing). The cause is not a routine deprecation: `arrayref` 0.3.5–0.3.9 were
> **all yanked**, and a 0.3.10 published 2026-08-20 07:15Z adds a *normal* dependency on
> **`proc-macro1`** — a crate that **404s on crates.io** and is a typosquat of `proc-macro2`. Upstream
> `droundy/arrayref` `master` is still 0.3.9 with no such dependency and no 0.3.10 tag: **the published
> artifact does not match its own source.** That is a compromised publishing account, and a proc-macro is
> the ideal payload because it runs at *compile* time. The chain is
> `arrayref → blake3 → bao → cairn-event`, i.e. the content-addressing and signing crate.
>
> **The exposure was worse than a lockfile pin suggests, and it is worth knowing why.** Two of three
> cargo trees were pinned; `extensions/cairn_pgx` had its `Cargo.lock` **gitignored**, so `cargo pgrx
> install` re-resolved on every CI run — and today's run took `arrayref 0.3.10` and tried to fetch
> `proc-macro1`, dying in 45s on a docs-only commit (the trace shows `blake3 v1.0.0` against the root's
> pinned `1.8.5`: a fresh resolve, not a locked one). It failed **closed** only because crates.io had
> already pulled the typosquat; in the window before that, it would have compiled an unknown proc macro
> **at build time** into the shared object that enforces the in-DB floor. It was also nondeterministic —
> the run twelve minutes earlier passed on a restored `rust-cache` lockfile. **Fixed in `5d23d0a`:** that
> lockfile is now committed (pre-incident resolve, arrayref 0.3.9) and its `.gitignore` says why it must
> not be re-added.
>
> Hold the pin. `cargo-deny` stays red on the yanked 0.3.9 and is **deliberately not worked around**. If
> crates.io deletes 0.3.10 and un-yanks the real versions, the gate goes green with no repo change —
> check before touching anything. See #445 for the full evidence and the options.

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
   (commands verified working) and record into a dated copy of `TEMPLATE.md`. Only the *write* half is
   measured (median 222 ms — `results/2026-08-03-node-tier-write-cost.md`, **PARTIAL** in its title for
   this reason). Slice 63 owes BOTH halves for registration (≤ 5 s to find an existing chart, ≤ 20 s to
   register a new one); the node-tier write-cost half is
   [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360) (nothing is wired; db/044's `gesture_kind`
   CHECK refuses a registration row until widened).
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`. The fixture
   chart deliberately carries a cross-patient line and an invisible group so the ADR-0060 warnings are
   exercised. Automating the DOM assertions is **#332** (needs a JS-toolchain decision: plain JS, no npm).
3. **Make two CI jobs REQUIRED status checks** ([#444](https://github.com/cairn-ehr/cairn-ehr/issues/444),
   admin-only) — "clippy + cargo test (cairn-gui)" (PR #343: the reference-UI workspace and its JS/Rust
   drift guard) and "cargo doc (API surface)" (#439). Both run on every PR; neither is in main's branch
   protection, so both can go red without blocking a merge. Match the job names exactly — a mismatch
   orphans the required check and blocks every PR silently. `CONTRIBUTING.md` carries the current state
   in a "jobs that run but do not yet block" table, **dated, because branch protection lives on GitHub
   and no gate can keep that table honest**. Note the doc gate is no longer *only* advisory: the
   root-workspace, `--features fixtures` and `cairn_pgx` doc builds all run as the last steps of the
   REQUIRED `test` job, so promoting `doc` now buys speed of signal, not coverage. Only cairn-gui's half
   still depends on an unrequired job.

**If a measurement falls outside its budget, that is the finding — file an issue; do not adjust the
budget to match.**

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
4. **[#370](https://github.com/cairn-ehr/cairn-ehr/issues/370)** — the clinical plane's copy of #228: a
   malformed `digest_hex` raises in the `22` class, which `cairn-sync` reads as transient and freezes the
   pull cursor on. An availability defect wearing a legibility defect's clothes.

**Standing gate:** whole-project review cycles repeat periodically, and there will be **no release for
clinical use before repeated review cycles pass cleanly.** Last full pass 2026-07-15
([report](code_reviews/2026-07-15-whole-project-architecture-review.md), findings #187–#217), **fully
closed**. A runnable clinical surface exists that has never been through one — include it next.

**The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
session holds the main repo. **Never start it alongside a human session**: they contend on one cargo lock
and one `test_serial_guard` advisory lock (a stray loop once stretched a session's suites from ~3 min to
~90 min).

> [!TIP]
> **A live IDE contends the same way, and it is not obvious.** rust-analyzer's `cargo check
> --workspace --all-targets` holds the shared `target/` lock, so a narrow `cargo test` blocks before it
> compiles, then times out. Fix is a scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the IDE. **The old
> "recreate cairn_test/2/3 after an `event_log` column add" note is OBSOLETE** — since #296 the suites
> build `event_log` rows by name via `jsonb_populate_record`, so the stale-column-order failure is
> structurally closed.

---

**Session date:** 2026-08-20 (a trap-clearing pass: #439/#382/#385/#381) · **Spec/ADRs:** v0.66 (through **ADR-0064**,
*admit the claim, withhold the power*; ADR-0063 gained erratum E1, **ADR-0064 gained erratum E1**) · **`SCHEMA_GENERATION`:** 49 (`db/049`) ·
**Phase:** architecture complete (every original §11 question closed); **first production clinical
surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** (full detail in ROADMAP + the ADR log + git):

- **Demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex /
  gender-identity · §4.3 address; karyotype resolved as a distinct field, ADR-0037, no code yet) · **the
  §5.2 advisory Python matcher** (in-DB veto floor, scoring core, veto-gated pipeline/blocking, the B3
  eval harness, compound blocking keys, volume generator, Fellegi–Sunter weight-learning).
- **The §5.7 identity core C1–C5** — link · human-accepted apply seam · auto-apply band · dispute ·
  identify · repudiate + the known-alias pool. The confirmed/unconfirmed/under-review contract is
  COMPLETE; C5+ `reattribute` waits on a clinical-note surface. **The §5.4 John-Doe subsystem** —
  slices A–D, finishers, photo/text evidence, the `enroll-human` ceremony CLI; §5.12 push-alert open.
- **The §5.3/§5.8 search-before-create funnel** (ADR-0061) — the registration act, its db/045 floor and
  retained-set projection, the advisory db/046 search, `cairn-patient-search`, two CLI verbs, John Doe
  re-expressed onto the same act — plus its **precedence rule** (#345, db/005 step 8b; `patient.created`
  retired in db/047, which handed registration the `patient_chart` chart-birth projection).
- **`clinical.medication` slices 1–6b** — assert/cease + the E1 reconciliation flag · bitemporal dose
  timeline · cross-thread reconciliation (ADR-0047) · attestation responsibility overlay (ADR-0049) ·
  per-field dose correction (ADR-0050) · inline `substance.coding` + two coding-overlay verbs (ADR-0059);
  with the **twin-check registry** (ADR-0048) and the **contributor-role vocabulary floor** (ADR-0051).
- **Born-sealed clinical bodies** (ADR-0052), confidentiality-capable since #231 pinned the unwrap-cert
  `kid` to `trust_peer` · **per-write human authorship** (ADR-0053 — grading half-live until #245) ·
  **the §5.9 stream COMPLETE through its read surface** (parts A/B, the authority floor, the operator
  surface — see ⇒ NEXT). **§5.9 enforces nothing beyond display/emission.**
- **The L3 reference UI** — `cairn-gui/`, a standalone workspace, dependency direction one-way GUI →
  crates. The iced shell FAILED the accessibility bar (spike 0004) and was **retired 2026-08-03**; today
  it is **`cairn-gui-tauri`**, a Tauri 2 window on one patient's medication chart (whole-list sign-off,
  per-row cease, plain-JS semantic HTML, no npm). The pane/routing/freshness state machine survived the
  retirement, is tested, and is **not wired**.
- **The med-list node tier** (Cairn's first clinical READ path + whole-list sign-off + two CLI verbs) ·
  **generic reprojection** (ADR-0057) · the **ADR-0056 admit-uninterpreted floor** + the **residual
  refusal contract** on the clinical plane (a deliberate refusal is penned verbatim, pinned,
  auto-released on repair, and a frozen watermark fails loud).
---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative; this section keeps only what a *next* session needs.

**2026-08-20 — a trap-clearing pass** (closes
[#439](https://github.com/cairn-ehr/cairn-ehr/issues/439), **#382**, **#385**,
[#381](https://github.com/cairn-ehr/cairn-ehr/issues/381); no ADR, no migration, SCHEMA stays 49). Four
small independent items — a red docs build and three §5.9 review follow-ons. Worth carrying:

1. **`cargo doc` was red on main, and that is what made it dangerous.** Two rustdoc errors meant the build
   only completed under `-A rustdoc::invalid_html_tags -A rustdoc::private_intra_doc_links`, under which
   every LATER rustdoc error is invisible too. The fix is the gate, not the edit. **The blocking copy
   lives in the REQUIRED `test` job** (root workspace + `--features fixtures` + `cairn_pgx`, as its last
   steps, so a rustdoc nit cannot abort the floor tests); the fast `doc` job duplicates the root build for
   feedback in under a minute, and cairn-gui's rides `gui`. Putting the root build only in `doc` was the
   review finding that mattered — **both of #439's defects were in the root workspace, i.e. in the one
   tree whose gate did not block** (#444). `RUSTDOCFLAGS=-D warnings` is set EXPLICITLY because only the
   root workspace denies warnings via `[workspace.lints]`. Both #439 defects are **warn-by-default
   rustdoc lints**, not hard errors — an earlier comment said otherwise, and believing it would make the
   `RUSTDOCFLAGS` line look redundant and un-gate the other two trees.
2. **A convention followed by 5 of 22 is worse than one followed by none.** `REVOKE EXECUTE … FROM PUBLIC`
   on the `cairn_check_*` validators is genuinely LOW severity, but a reader could not tell an oversight
   from a decision. What was bought is that it is **checkable**:
   `crates/cairn-node/tests/floor_execute_grants.rs` asserts it over the `pg_proc` CATALOGUE, so a future
   migration is covered the moment it loads. **Read `proacl`, never assume: a NULL ACL is the PERMISSIVE
   case** (Postgres's default for a function is EXECUTE to PUBLIC), and inverting that polarity makes the
   guard pass by seeing nothing. Its second test ratchets the half that IS load-bearing — the projection
   appliers — and reads them from the **`cairn_projection_apply` REGISTRY, not from the `_apply` name
   suffix**: the registry holds 21 and exactly one, `medication_dose_seed_initial`, does not carry the
   suffix, so the first draft's name pattern could not see it at all (it is revoked, so there was never an
   exposure — the ratchet simply had a blind spot). Where a family HAS an authoritative list, read the
   list. db/005 states the two-reasons-one-statement distinction once.
3. **Every new guard was mutation-tested before being trusted**, because two of the three (#385's type
   short-circuit, #381's cross-chart pin) are exactly the shape the 08-19 review lesson warns about.
   #385's short-circuit is pinned by seeding a **decoy** `medication_statement` row carrying a note's own
   content address; #381's cross-chart block seeds an **attested** withdrawal, since an unattested one
   fails to lower for an unrelated reason and would pass with the pin deleted.
4. **`cairn_event_thread` no longer asks a note for its medication thread**, and the five projections now
   index `content_address`. No query-plan assertion, deliberately — at test scale Postgres correctly
   prefers a seq scan, so an `EXPLAIN` test would fail on a *correct* index. **The magnitude of the win is
   still unmeasured on volume data**; that is a real residual, not a closed question.
5. **THE LESSON OF THIS PR, found in review: an optimisation removed a redundancy that was load-bearing,
   and the comment explaining it asserted the exact opposite.** The #385 short-circuit's first draft said
   widening §10b's list could only over-protect — "there is no spelling of that list that makes this
   leak". Measured, it is the reverse: §11's conservative bound is gated on the NEGATION of the same
   predicate, so a type added to the list is EXCLUDED from the bound, and after #385 it also stops
   resolving. All three thread arms fall silent and a standing `sequestered` grade reads back as
   `('routine','none')`. **Before #385 the identical edit was harmless**, because §11's resolved arm was
   an independent net that never consulted the predicate. Two things to carry: (a) when an optimisation
   makes two code paths share a predicate, ask what redundancy that share destroyed; (b) the comment did
   not merely mislead — it told the reader that
   `no_medication_event_type_is_classified_thread_free` was a quality guard, when it is one of only two
   things standing between a six-line `LIKE` edit and silent protection loss across every medication
   thread. **A wrong safety argument is worse than none: it disarms the guard it describes.**

**2026-08-19 (later) — §5.9 type design** (closes
[#387](https://github.com/cairn-ehr/cairn-ehr/issues/387); opened
[#439](https://github.com/cairn-ehr/cairn-ehr/issues/439), **closed 2026-08-20**; no ADR, no migration).
Four closed sets get one definition each. **`WinningSubject`** fuses `chart_source` +
`chart_content_address` into a sum type keyed on the ADDRESS, making ADR-0062 erratum E6 structurally
unrepeatable — its payload fields are **private**, so its two constructors really are the only producers
(the first draft left them public while the doc claimed otherwise). **`GRADE_*` are consts and
`Provenance` is an enum, and the asymmetry is the point**: ADR-0062 decision 2 argues for `grade` being
OPEN, while `source` closes because of what **db/048 does** — stores it, reads it from no query, no
projection, no rank function. Builder-side only; db/048 mints a THIRD value itself (`'unreadable'`), so a
read model must keep `source` open text. **`SubjectKind` is generated from one table, not hand-listed.**

**⇒ THE REVIEW LESSON, and it generalises far past that slice: a guard defined over the list it guards is
not a guard.** Every finding there was that shape and none was a behaviour bug — the code was right;
three of four headline claims were true only in the comments. `assert_eq!(SubjectKind::ALL.len(), 3)` over
an `[SubjectKind; 3]` compared a compile-time constant to its own literal and could not fail; `from_row`
was "the ONE place" over **public** fields. **When reviewing a guard, ask what independent source it
checks against. If the answer is "itself", it is documentation wearing a test's clothes.** (The 08-20 pass
applied this by mutation-testing every new guard before trusting it.)

Also recorded: **#387's premise did not survive contact with the code** — it cited ~69 grade literals
across 7 files; a survey found **exactly one in production code** workspace-wide. The consts are
documentary, not de-duplication, and are pinned only where pinning is possible (`sensitivity_ladder.rs`
feeds all four to `cairn_sensitivity_rank` live, since a Rust-only assertion compares a const to itself).

**2026-08-19 — the withdraw read-back** (closes
[#435](https://github.com/cairn-ehr/cairn-ehr/issues/435); opens
[#436](https://github.com/cairn-ehr/cairn-ehr/issues/436); no ADR, no migration). Slice 69's read-back
comment claimed *"both orchestrators"*; only one had it, so `sensitivity-withdraw` printed plain success
over a withdrawal that removed nothing. New `sensitivity/readback.rs` reports **two independently
observed facts, never merged** — which worklist arm (accountability) and what this node can say about the
target (effect). Three things to carry:

1. **The effect fact needs its own query.** db/048's `inert` arm merges *"nobody accountable"* with *"the
   target has not replicated here yet"* — its own comment says so — and only reading
   `sensitivity_assertion` separates them. Resolved against the target's OWN subject, never the
   chart-wide grade.
2. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`.** ADR-0064's
   KNOWN GAP: a withdrawal mis-stamped with the wrong chart's `patient_id` names a real assertion living
   elsewhere, and `cairn_sensitivity_standing` is patient-scoped on both sides (load-bearing — else chart
   B strips chart A), so the target IS genuinely absent from this chart's standing set. A naive membership
   test therefore reports the withdrawal **effective** — a precise untruth in the *reassuring* direction,
   on a confidentiality surface. Caught in this slice's own first draft; it now has a test.
3. **#436 is the residual, and it is a visibility problem, not a door problem.** The same shape arriving
   by REPLICATION is invisible to every §5.9 read surface. Refusing at apply would fork the event set, and
   *"on another chart"* is indistinguishable from *"not arrived yet"* on a node holding neither — so any
   block built for it must say WHICH it is claiming, rather than merging the pair the way `inert` does.

All five production mutations tried against the new tests were killed.

**2026-08-18 — Slice 69: the §5.9 operator surface** (closes #388, #383, #421; **ADR-0064 gains erratum
E1**; no new ADR, no new migration). Three slices had shipped a §5.9 mechanism and no way to look at it;
`patient-sensitivity <chart>` is now the one query that tells the whole truth. Carry:

1. **NAME, NEVER COUNT.** A count cannot separate **custody-blind** from **genuinely empty** — the one
   question the line exists to answer. A later "simplification" back to a count is a regression.
2. **A chart-scoped definer, not a table grant — granted to BOTH group roles.** `event_deferred` is
   granted to `cairn_node`, NOT `cairn_agent`, so db/043 gained `cairn_patient_deferred_sensitivity(uuid)`
   and its test pins **why it exists** (asserting `cairn_agent` genuinely lacks `SELECT`). The first draft
   granted EXECUTE to `cairn_agent` alone and called it "the runtime role" — it is not; the only
   role-membership grant in the tree is `GRANT cairn_node TO <login role>` (`db.rs`). **#425** owns which
   role the runtime should be.
3. **The report declares what it cannot contain** — ADR-0064's invisible cross-chart withdrawal, #414's
   unconsumed `RAISE WARNING`, #434's prefix filter — each asserted by a test over an **empty** list: the
   case where silence is most convincing and most wrong.
4. **Two Rust traps.** A type named only inside `#[cfg(test)]` compiles clean under `cargo test --lib` and
   fails the integration build under `-D warnings` — use `--all-targets`. And `search_path_pg_temp.rs`
   compares `rows.len() >= PINNED_TODAY` deliberately: a 26th definer needs **no** number moved.

**The PR #433 review landed a fix wave.** Three lessons generalise well past the slice. **(a) A union view
whose arms mean opposite things must never get one summary sentence:** the worklist's `stranger-attested`
arm DID take effect, yet the draft counted it under *"N withdrawal(s) did NOT take effect"* — telling the
operator a completed, unaccountable removal of protection had not happened. One header per reason now.
**(b) A proxy is not a fact when the fact is exactly determinable:** custody-blindness was inferred from
`standing.is_empty()` — wrong both ways, since grading is opt-in (#383 surviving inside its own fix) and
partial custody is invisible to any proxy; `event_log` keeps the sealed row, `event_clear` is what this
node can open, and db/048 §11b measures the difference. **(c) Peer text is not display text:**
`node_origin`/`event_type`/`grade` are unconstrained `TEXT` copied verbatim from a peer's body, and a
newline forged a whole line — `render.rs::peer()` Debug-escapes every such field.

**2026-08-16 (later) — the pinned `search_path` that pinned nothing** (closes
[#426](https://github.com/cairn-ehr/cairn-ehr/issues/426); 21 function headers gained `, pg_temp`).
**Live data loss at both owner-rights write doors, not hygiene:** `SET search_path = public` does not
exclude the session temp schema, so with a decoy `event_log` in place `submit_event` and
`apply_remote_event` each **RETURNED SUCCESS while the owner-privileged INSERT landed in the caller's temp
table**. A pinned path must **deny the temp schema the first look** (not merely "end in `pg_temp`"), and
the guard is over `pg_proc`, not a name list. Still open: **#430** (~100 unpinned invoker-rights
functions; `cairn_patient_has_events` is safe only by *inheriting* `submit_event`'s path) and **#431**
(`cairn_execute_shred`, catalogue-only coverage — a diverted shred would report an erasure that never
happened).

**2026-08-16 — two live §5.9 leaks, one per plane** (closes #412 and #405; ADR-0063 erratum E1). Both were
**a guarantee asserted in a comment that the code did not provide** — and the review of the fix reproduced
that exact shape in its own prose. The column-grant half is carried in ⇒ NEXT (residual **#427**). The
other half generalises: **a parameter name is not a security property** —
`classify_authorship_confidence(&body.contributors, &body.signer_key_id, None)` compiled, read naturally,
and graded a forgery `Attested`. Both key arguments are now a `VerifiedKid` newtype (mint-site allowlist
unpinned: **#428**). **`attester_key` alone is NOT proof** — db/020's deferred arm stores a peer's token
unverified, which is why SQL's R1 pairs it with `cairn_attestation_vouched`.

**2026-08-15 — Slice 68: claim authority at the apply door** (closes #380, discharges #405 part 2;
**ADR-0064**, spec v0.66). Full reasoning is ADR-0064's nine decisions; the floor itself is summarised in
⇒ NEXT. Two properties to carry: it **gates effect, never admission**, and only in the withholding
direction — a claim below the bar still lands and converges, so no fork (the **#342** trap); and
**computing the verdict at read cuts both ways** — both routes resolve through `actor_current`, so
revoking someone *after* their withdrawal landed silently re-raises the grade (**#409**), while the
Rust↔SQL authority mapping separately diverges on two shapes (**#408**, root cause **#413**). Also:
**flag what cannot self-heal, view what can** — the withdrawal worklist is a VIEW, `safety_overclaim_flag`
a LEDGER.

**The PR #410 review** ran six agents plus mutation testing on a live PG18; **7 of 11 production-code
mutations survived a green suite** — the review in one statistic. What generalises: **(a)** R2's
self-identity equality was completely unpinned (`c.actor_id = t.actor_id` → `TRUE` left the suite green
and reopened #380 in full, because every un-attested fixture used the *device* as both asserter and
withdrawer; pinning it needs two DISTINCT human actors). **(b) `EXCEPTION WHEN OTHERS` does not catch a
statement timeout** — PostgreSQL's `OTHERS` excludes `query_canceled` (57014), so a blanket handler let a
timeout abort `submit_event`, reproducing the incident ADR-0063 decision 8 exists to prevent. **(c)**
Comments asserting guarantees the code does not provide were the largest single class, and the #405 *fix*
re-committed it. Filed, not fixed: **#413** · **#414** (the overclaim ledger rests on a `RAISE WARNING`
nothing consumes) · **#415** (`stranger-attested` measures the SIGNER, so it fires on routine care —
**expect noise**) · **#416** (a sealed withdrawal is inert and invisible) · **#417** (wrong ADR line
citations; ADRs are immutable) · **#418** · **#419** · **#420** · **#422**.

**2026-08-14 — Slice 67: the §5.9 safety projection, part B** (closes #375, discharges #294; **ADR-0063**,
spec v0.65, SCHEMA 48→49). **The seal boundary is the coarsening boundary:** precise `{class, severity}`
travels sealed with the body, a grade-chosen **rung** rides the envelope in the clear, so
*coarsen-but-survive* after a crypto-shred is structural. **Two coarsenings, load-bearing for DIFFERENT
reasons:** emission binds a peer's raw-SQL client; read answers a peer that legitimately emitted a finer
rung, and **read coarsening is a rendering choice, not a floor**. **`safety_class_map` ships EMPTY** — the
seam drugref plugs into. The PR #403 review also fixed **#404** (`cairn_prospective_sensitivity`'s thread
arm had diverged from db/048, so `p_thread` was inert). Open: **#407** (the sealed precise claim is read by
nothing), **#406** (no supersession — a ceased drug warns forever), **#394**, **#395**, **#397**, **#398**,
**#399**, **#400**, **#401**, **#402**.

**2026-08-11 → 08-02 — Slices 66, 65, 63, 61+62, condensed** (ROADMAP carries each in full).
**Slice 66** (closes #231; ADR-0052 erratum E1) pinned the unwrap-cert kid to `trust_peer` (db/007) —
before it, any self-signed cert reaching the serve port obtained read-custody of every non-shredded
sealed body. **Withhold the key, never the bytes**; refusing the bytes would fork the event set, and
repair is TWO steps (`pull --full`, then `cairn_reproject()`). Same day (PR #390) cargo-deny's v2
`unsound = "none"` default let an unsound advisory pass in silence; `unsound = "all"` is now set in
**both** `deny.toml` trees, with **#389** ignored with a reason and an expiry. **Slice 65** (ADR-0062,
spec v0.64) — its unknown-ranks-MAX trap is in the ⇒ NEXT callout, and the grade being **node-relative**
is in the part C bullet. **Slice 63** (ADR-0061, spec v0.63) — carry into any registration work: **the
attestation NAMES the displayed candidates, it does not count them** (*was the duplicate on screen when
the clerk clicked create?* has opposite fixes for yes and no, and `N = 3` cannot separate them);
follow-ons #346–#357, #359–#362 in ROADMAP, §1.2 write-cost half **#360**. **Slices 61+62** — three
lessons: **a displayed row is a GROUP, an attestation is a THREAD** (ADR-0047/ADR-0049 — nearly every
defect lived on that seam); **a unit-tested safety control can still be defeated by the surface that
calls it** (the idle re-lock never fired because a shared accessor counted every poll as activity, every
`SessionKey` unit test passing — **test the path the product actually calls**); and **a compensating
control outside CI is not a control** (`cairn-gui` is a separate workspace `cargo test --workspace` never
covered; the `gui` job now does, ⚠️ still not REQUIRED).

> [!IMPORTANT]
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md):
> *partial validity — a defect on one line never invalidates another.*** Read before any
> composite-clinical-object work: **the system may fail to record an order, but it may never cancel
> one.** Hold decision 2 (partial completion must be reported, never implied) and decision 7 (check the
> transaction boundaries).

**Four repo conventions these runs learned the hard way:**
- **Guard before connect.** DB-gated tests take `db::test_serial_guard(&base)` *before*
  `connect_and_load_schema`. Every existing suite does this in execution order.
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
demographics + matcher · identity/John-Doe/medication · the five-priority review course → ADR-0051–0058 ·
ADR-0059 + medication 6a/6b · the ADR-0056 admit-uninterpreted floor · floor determinism (#75) ·
tech-debt-loop launch.

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in
[`scratch/ui-sketches/easygp-consult-screen-inventory.md`](../scratch/ui-sketches/easygp-consult-screen-inventory.md)
and `easygp-editing-area-inventory.md` (source screenshots git-ignored under
`docs/untracked_for_brainstorming/` — real photos, **never commit or publish**). Headline: easyGP's six
editing-area invariants ≅ Cairn's event envelope near line-for-line. **Open:** co-author questions in
that note §7; results-inbox screenshots pending — three-zone vs two-pane rides on them, **don't
improvise it**. **Scope:** the easyGP co-author may lead GP-facing GUI, HH designs ED & ward; the
role-manifest layer is the seam (ADR-0021).

**Status of this file:** Disposable working scaffolding, **not** a source of truth. Regenerate at the end
of each session, and **keep it under 500 lines** — its value is inversely proportional to its length
(#368). If it disagrees with the canonical docs, **the canonical docs win.** The *why* lives in the ADR
log, the *what* in the spec; this file carries only what lives *between* them — current build state, open
threads, time-sensitive items.

---

## Read these first (the durable state)

- **`docs/spec/index.md`** — canonical architecture spec (mission prose + document map + spec version).
  One file per aspect; cross-refs like *§5.7* stay valid inside the aspect file.
- **`docs/spec/decisions/README.md`** — the **ADR log index** (the *why*). Numbered, dated, **immutable**
  — a reversal is a new superseding ADR, a factual correction is an appended erratum. **Read the relevant
  ADR before reopening a settled question.**
- **`docs/ROADMAP.md`** — the foundation build order, *below* the policy/GUI line, plus the per-slice
  narrative. Disposable scaffolding like this file; the spec/ADRs win on any disagreement.
- **`docs/spikes/`** — 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor
  — C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓); 0004 (iced UI — FAIL on a11y →
  Tauri 2). **`docs/case-studies/`** — 0001 (2026-07-11): 16 Australian GP-software failure modes, all
  absorbed, **0 new architecture**. **`docs/ecosystem/`** — 0001, 0003. **`docs/principles/`** —
  mission/governance; root **`README.md`** repeats the founding principles.
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
  `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace).
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (2026-06-21, first implementation of
  [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)) —
  `cairn-node` (Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS pinned to the trust set, set-union
  `node_event` sync) + the `db/007` doors with a deny-all admission gate; genesis-stable `node_id`.
  **Every honest gap declared at build time is CLOSED**, including all four ADR-0026 durability slices
  A–D — only optional escrow *rungs* (Shamir/QR/TPM) remain. The `localstate` read/apply **seams** are
  where the clinical tier plugs DEKs/drafts/config.
- **Dual-identifier discipline** (ADR-0031) — the canonical plane (UUIDv7 + multihash) is the *only*
  identifier on the wire/in signed bodies; the projection plane may intern to node-local `bigint`
  surrogates (`db/008` + the leakage guard). The load-bearing guarantee is the typed signed plane.
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`). Connection strings and the
  DB-slice runner are under Open threads → Test env.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels and drives `/techdebt-next` one
  fresh headless session per issue until the ready backlog is dry (`tail -f ~/.cairn-loop/run.log`).
  Auto-merge **ENABLED**; **works unattended** (12 PRs across two runs); **stopped** by maintainer
  decision — see ⇒ NEXT. Cold-start ladder: `--dry-run`, `--max-issues 1` watched, then unbounded. Live
  gaps: **#326**, **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts C/D** ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) — A, B and the authority
  floor all shipped (Slices 65/67/68); **C is unblocked**, its open decision is the dial question (⇒ NEXT).
  Related: **#235** (shred authorization policy hooks), **#236** (FTS/RAG must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059 fully implemented 2026-07-28). **Next
  candidates:** the **drugref term→anchor lookup** (⇒ NEXT item 2); fuzzy/automatic reconciliation + a
  Tier-A drug dictionary; structured sig/frequency (lands with prescriptions); correcting a dose event's
  *effective date* on the statement-level `started`. **Cross-cutting debt: #185** (cross-thread
  correction *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision).
  Spine to reuse: `db/031`–`db/033`, `db/041`, `db/042` + `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine: `db/010`–`db/030` +
  `cairn-event::demographics`; everything under "Built so far" is DONE). **Next (B3
  measurement-driven):** a **large hand-crafted gold set** to re-run the learner for authoritative
  magnitudes (slice 24's is a PoC on synthetic data); locale comparator packs; the hub-tier duplicate
  sweep; proposal retraction; richer §7.5 matcher-actor determinants. **Next identity:** C5+
  `reattribute` (**waits on a clinical-note surface**); the §5.12 push-alert. Deferred: **#168**
  (entity→role-actor 1:many), **#287** (sweep re-scores standing orphans); unfiled ones are in ROADMAP's
  "Still open from slices 36–56".
- **Test env:** DB-gated + matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532
  user=hherb dbname=cairn_test"` (PG18+cairn_pgx), plus `CAIRN_TEST_PG2`/`PG3` (`cairn_test2`/`3`, same
  cluster) for the multi-node convergence suites — without them those **self-skip and cargo counts them
  as passed**, so a workspace count alone cannot distinguish skip from pass (CI sets all three, #199).
  Matcher integration: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest`; the pure suite is
  dependency-free (`uv run pytest`) — uv, never venv/pip. The `db/tests/*.sql` **mirrors run only via
  `scripts/run-db-sql-tests.sh`**, which drops, recreates and marks a throwaway `cairn_sqltest`: since
  #169 each mirror refuses a database lacking the `cairn_scratch_database` marker, because the mirrors are
  destructive (eight commit; `017` drops constraints). `scripts/run-db-gated-tests.sh` runs the mirrors
  *and* the full workspace with all three connection strings baked in — the one command for the DB slice
  of the local gate. Local gap: [#314](https://github.com/cairn-ehr/cairn-ehr/issues/314) (it does not run
  the matcher DB-gated pytest suite; CI does).
- **Clinical case-mining** — historically the highest-signal generative mode; the event-overlay +
  key-custody + actor primitives have absorbed every case so far without new architecture. Bring a real
  ED/hospital failure mode; record in [`docs/case-studies/`](case-studies/README.md). Open action items
  from Case 0001: **① re-affirmation-without-change currency** ([#163](https://github.com/cairn-ehr/cairn-ehr/issues/163));
  **② open-loop/obligation** (order/recall/referral with no closing ack) may warrant a named projection,
  surfaced by salience not a modal; **③ impossible-vs-uncertain** constraint rule for the in-DB floor.
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already
  advanced across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#9-bet-b--results-raspberry-pi-5--8-gb-2026-06-25--pass-with-two-honest-caveats)):
  **PASS twice** (clean 2026-07-07 re-run, PG 18.4 + NVMe HAT, both caveats resolved — B1 p95 **3.99 ms @
  2,004,000 events**, 13× under budget; B4 confirms ADR-0015's BLAKE3 default). **Remaining:** fold the
  un-caveated B4 number into ADR-0015 to drop "provisional" from the blob-digest line, and
  [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (reproject bench on the Pi rig).
- **easyGP session** — port the [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  deferred items with live easyGP schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon (validates ADR-0001 from production). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **easyGP GUI-mining continuation** — more consult-screen/module screenshots incoming from the co-author;
  they should answer most of the remaining §4.4 open questions in
  `scratch/ui-sketches/easygp-consult-screen-inventory.md` and open the **results/inbox design session**
  (three-zone vs two-pane is parked there — don't improvise it).
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection
  per slice (the production object-store tier). The §8.2 availability + windowing/resume work shipped.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** stewarding legal entity & jurisdiction (German
  Stiftung/Verein, US 501(c)(3), or an umbrella) — deferred until momentum/funding geography is clearer;
  formal trademark / wordmark registration — principle recorded (stewardship doc), legal instrument
  deferred.
- **CLAUDE.md carries the working context in full and is loaded every session** — the working
  conventions, the twelve founding principles (the first four being the lens for every design choice),
  and the §9 defect-blast-radius language rule. Not restated here; canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr`
  org; `cairn-ehr.org`+`.com`; PyPI/crates.io/npm `@cairn-ehr` placeholders).
