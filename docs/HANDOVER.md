# HANDOVER — Cairn

## ⇒ NEXT

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems; A, B and
the cross-cutting authority floor are now built.** Read [ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) and
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) before touching the rest; do not re-derive their decisions.

- **Part A — the sensitivity stream** (Slice 65, ADR-0062): graded append-only assertions over an event /
  a thread / a whole chart; effective grade is the **max** over all three. Computes and reports only.
- **Part B — safety-projection emission** (Slice 67, ADR-0063, `SCHEMA_GENERATION` 49):
  the grade now *does* something. The precise `{class, severity}` is captured **pre-seal** and sealed with
  the body; a **rung** chosen by the standing grade rides the envelope in the clear. Discharges
  [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294). Still enforces nothing — it emits a *signal*.
- **The authority floor — admit the claim, withhold the power** (Slice 68, **ADR-0064**, spec v0.66,
  closes [#380](https://github.com/cairn-ehr/cairn-ehr/issues/380), discharges
  [#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 2, gives
  [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245) its first SQL counterpart — NOT its
  "mirror", a word both `contributor.rs` and ADR-0064 explicitly retract, and NOT its display half,
  which stays open): a protection-removing claim
  (a grade withdrawal, an emitted safety rung) now takes effect only when a human this node can hold
  responsible stands behind it — one predicate (`cairn_claim_authority`, db/005) consulted at exactly one
  site (the `NOT EXISTS` in `cairn_sensitivity_standing`, db/048), so display coarsening, safety-rung
  emission and part C's dial below all inherit it structurally. **This is the floor part C now keys on** —
  read it before touching sequester, rather than #380 itself (closed with PR #410, merged 2026-08-16).
- **⇒ Part C — sequester / custody narrowing** ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)):
  Slice 66 (#231) pinned custody to admission and Slice 68 (ADR-0064) closed the un-attested-strip hole a
  grade-keyed dial would otherwise have inherited. **What remains is the dial question, sharpened by
  ADR-0064 §8's finding**: a custody dial *derived from* the effective grade is only as strong as its
  most-custodial holder — the grade is node-relative (ADR-0062 decision 9), so a well-custodied peer
  legitimately computes a *lower* grade and hands out the DEK on it, and no amount of authority hardening
  changes that. An **explicit custody act** (a signed `custody.narrowed`-shaped event, not a value derived
  from the sensitivity stream) has no such property. **This is an input to #376, not a decision taken —
  do not treat it as settled.**
- **Part D — break-glass** ([#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)): audited key-*use*,
  partition-honest. Blocked on C.

**Slice 68 shipped two surfaces with no reader.** `sensitivity_withdrawal_worklist` (view) and
`safety_overclaim_flag` (ledger) are tested and GRANTed to `cairn_agent`, but nothing in the workspace
displays either — [#388](https://github.com/cairn-ehr/cairn-ehr/issues/388) territory. ADR-0064's §1.2
budget (*"why didn't this withdrawal take effect?" in one query, no raw SQL*) is **owed, not met**.

**2026-08-16 closed one live §5.9 leak and narrowed the other**
([#412](https://github.com/cairn-ehr/cairn-ehr/issues/412) closed;
[#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 1's *convenient path* only) — see the
session entry below. Two things to carry before touching either plane:
**`REVOKE SELECT (column)` is inert while a table-level grant stands**, so `cairn_agent` now holds an
explicit 23-column grant on `event_log` that omits `safety`, and **adding a column to `event_log` now
requires granting it in db/049 section 8** (fail-closed; `safety_read_grants.rs` fails by column name to
tell you so). And — the correction that matters most — **that grant is cost-raising, not a floor**: the
column is a copy of a clear field of the signed body, so `cairn_body(signed_bytes) -> 'safety'` still
returns it uncoarsened to the same role ([#424](https://github.com/cairn-ehr/cairn-ehr/issues/424)), and
the runtime login role is a member of `cairn_node`, which keeps the table-level grant
([#425](https://github.com/cairn-ehr/cairn-ehr/issues/425)). Do not cite db/049 section 8 as a
confidentiality boundary; ADR-0063 decision 2 (emission-time coarsening) is the one that binds.

Slice 65's own follow-ons: **#374** (thread resolution resolves only a thread's *current head* — erratum
E4 narrows it), **#378** (the withdrawal rationale is clear text forever and replicates — the UI must warn
at entry today), **#379** (the grade in the twin), **#381** (db/tests/048 mirror parity), **#382**
(`REVOKE EXECUTE` on the `cairn_check_*` family), **#383** (`chart_sensitivity` reads
`medication_statement`, so a **custody-thin node** — the offline tier this project exists for — prints "no
medication threads" while honouring standing thread-scoped grades), **#385** (index `content_address` on
the five medication projections), **#387** (type design) and **#388** (the operator surface is blind to
withdrawals, deferred grades and custody-less charts). **#386 is now half-closed** — db/049's subset test
*drives* it; db/048's still does not.

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

**Three things the med-list slice still owes are HUMAN acts and cannot be done by an agent:**

1. **The §1.2 time budget is still a seeded figure, not a measured one.** Follow
   [`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md)
   (its commands are verified working) and record into a dated copy of `TEMPLATE.md`. Only the
   *write* half is measured so far — median 222 ms, in
   `results/2026-08-03-node-tier-write-cost.md`, which says **PARTIAL** in its title for this reason.
   Slice 63 owes BOTH halves for registration (budget: ≤ 5 s to find an existing chart, ≤ 20 s to
   register a new one) — the interactive half by the first runnable surface, and the node-tier
   write-cost half as [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360) (nothing is wired;
   db/044's `gesture_kind` CHECK refuses a registration row until widened).
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`. The fixture
   chart deliberately carries a cross-patient line and an invisible group so the ADR-0060 warnings are
   exercised. Automating the DOM assertions is **#332** and needs a JS-toolchain decision (plain JS, no
   npm, no bundler).
3. **Make the `gui` CI job a REQUIRED status check.** "clippy + cargo test (cairn-gui)" (PR #343) gates
   the reference-UI workspace and its JS/Rust drift guard, but only a repo admin can add it to main's
   branch protection; until then it can go red without blocking a merge. (Match the job name exactly, per
   the warning in `rust.yml`.)

**If either measurement falls outside its budget, that is the finding — file an issue; do not adjust the
budget to match.**

**The other build candidates** (any of them can be picked up next; nothing blocks a choice):

1. **The registration/search UI slice** — now that the node tier exists, the picker is the wrong-chart
   affordance paper has and the med-list window does not. **Constraint from Slice 63:** the picker must
   **open** a chart, never *retarget* an open window — retargeting re-creates the §5.8 item 4 / §5.11
   windowing misfile that possession semantics exist to prevent. Also wires the kept-but-unwired
   pane/routing/freshness state machine.
2. **The drugref term→anchor lookup** — the §9 *advisory* tier, and what actually closes the
   **coded↔uncoded** duplicate case ADR-0059 decision 5 deliberately leaves open. Needs a design
   decision first: the cross-service connection model. The slice-6a/6b source guard keeps the trusted
   surface drugref-free and must stay passing. **Slice 67 gave it a second consumer:** `safety_class_map`
   is the empty seam drugref would populate, and today no node has any class knowledge at all.
3. **The node/actor plane's two divergences.** `db/007` still fail-closes on an unmappable type (**#301**)
   and still skips-and-advances a verifiable refusal where the clinical plane now pens (**#268**).
   **Neither is a symmetric fix:** `node_event` is type-shaped, so #301 needs a carried-not-interpreted
   row shape plus an audit of every trust projection; #268 needs the door to tell "not-for-me trust-graph
   deny-all" from "genuinely refused history", or the pen fills with steady-state traffic. Both
   `loop:blocked`.
4. **[#370](https://github.com/cairn-ehr/cairn-ehr/issues/370) — the clinical plane's copy of the #228
   defect.** A malformed `digest_hex` in an attachment reference raises in the `22` class, which
   `cairn-sync` reads as a transient fault and freezes the pull cursor on. Same shape as PR #371, one
   plane over: an availability defect wearing a legibility defect's clothes.

**Standing gate:** whole-project review cycles repeat periodically, and there will be **no release for
clinical use before repeated review cycles pass cleanly.** Last full pass 2026-07-15
([report](code_reviews/2026-07-15-whole-project-architecture-review.md), findings #187–#217), **fully
closed**. A runnable clinical surface exists that has never been through one — include it next.

**The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
session holds the main repo — safe to re-run when the repo is free (`tail -f ~/.cairn-loop/run.log`).
**Never start it alongside a human session**: they contend on one cargo lock and one `test_serial_guard`
advisory lock (a stray loop once stretched a session's suites from ~3 min to ~90 min).

> [!TIP]
> **A live IDE contends the same way, and it is not obvious.** rust-analyzer's `cargo check
> --workspace --all-targets` holds the shared `target/` lock, so a narrow `cargo test` blocks before it
> compiles, then times out. Fix is a scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the IDE. **The old
> "recreate cairn_test/2/3 after an `event_log` column add" note is OBSOLETE** — since #296 the suites
> build `event_log` rows by name via `jsonb_populate_record`, so the stale-column-order failure is
> structurally closed.

---

**Session date:** 2026-08-16 (the two §5.9 leaks) · **Spec/ADRs:** v0.66 (through **ADR-0064**, *admit the
claim, withhold the power*; ADR-0063 gained erratum E1) · **`SCHEMA_GENERATION`:** 49 (`db/049`) ·
**Phase:** architecture complete (every original §11 question closed); **first production clinical
surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** (one line each; full detail in ROADMAP + the ADR log + git):

- **Demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex /
  gender-identity · §4.3 address; karyotype resolved as a distinct field, ADR-0037, no code yet).
- **The §5.2 advisory Python matcher** — in-DB veto floor, scoring core, veto-gated pipeline/blocking,
  the B3 eval harness, compound blocking keys, volume generator, Fellegi–Sunter weight-learning.
- **The §5.7 identity core C1–C5** — link · human-accepted apply seam · auto-apply band · dispute ·
  identify · repudiate + the known-alias pool. The confirmed/unconfirmed/under-review contract is
  COMPLETE; C5+ `reattribute` waits on a clinical-note surface.
- **The §5.4 John-Doe subsystem** — slices A–D, finishers, photo/text evidence, the `enroll-human`
  ceremony CLI. Still open: the §5.12 push-alert.
- **The §5.3/§5.8 search-before-create funnel** (ADR-0061) — the registration act, its db/045 floor and
  retained-set projection, the advisory db/046 search, `cairn-patient-search`, two CLI verbs, John Doe
  re-expressed onto the same act — plus its **precedence rule** (#345, db/005 step 8b; `patient.created`
  retired in db/047, which handed registration the `patient_chart` chart-birth projection).
- **`clinical.medication` slices 1–6b** — assert/cease + the E1 reconciliation flag · bitemporal dose
  timeline · cross-thread reconciliation (ADR-0047) · attestation responsibility overlay (ADR-0049) ·
  per-field dose correction (ADR-0050) · inline `substance.coding` + two coding-overlay verbs (ADR-0059);
  with the **twin-check registry** (ADR-0048) and the **contributor-role vocabulary floor** (ADR-0051).
- **Born-sealed clinical bodies** (ADR-0052) — confidentiality-capable, not merely an erasability
  substrate, since #231/Slice 66 pinned the unwrap-cert `kid` to `trust_peer`.
- **Per-write human authorship** (ADR-0053 — grading half-live until #245; its Rust grader now takes
  proof-carrying inputs, #412).
- **The §5.9 stream:** part A (ADR-0062 — graded append-only assertions + effective-grade read model +
  three CLI verbs) · part B, the safety projection (ADR-0063 — `db/049`, precise class sealed with the
  body, grade-chosen rung in the clear, an **empty** `safety_class_map`, `patient-safety`) · the
  cross-cutting **authority floor** (ADR-0064 — one predicate, one site). **Enforces nothing beyond
  display/emission** — sequester (part C) is next.
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

**2026-08-16 — two live §5.9 leaks, one per plane** (closes
[#412](https://github.com/cairn-ehr/cairn-ehr/issues/412); narrows
[#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 1 — part 2 was Slice 68; no ADR and no
migration, `SCHEMA_GENERATION` stays 49; **ADR-0063 gains erratum E1**). Both defects were the same shape:
**a guarantee asserted in a comment that the code did not provide** — and the review of this branch found
the SQL fix had reproduced that exact shape in its own prose, which is the fourth item below. Four things
to carry:

1. **A column-level `REVOKE` cannot narrow a table-level `GRANT`.** Postgres tracks the two separately, so
   `REVOKE SELECT (safety) ON event_log FROM cairn_agent` is **inert** while db/005's
   `GRANT SELECT ON event_log` stands — and a table grant covers every column added later, which is how
   the runtime role could read the uncoarsened safety signal raw. db/049 section 8 now drops to an
   explicit column grant; `cairn_event_safety` / `cairn_patient_safety` became `SECURITY DEFINER` so the
   sanctioned read still works. **The two halves are one fix** — the definer clause alone closes nothing,
   the revoke alone breaks the read path (mutation-checked both ways).
2. **The fail-closed cost is real and deliberate.** A future `event_log` column is unreadable by
   `cairn_agent` until db/049 section 8 grants it. That is the loud failure the previous inheritance
   silently avoided; `safety_read_grants.rs` names the missing column when it happens.
3. **A parameter name is not a security property.** `classify_authorship_confidence(&body.contributors,
   &body.signer_key_id, None)` compiled, read naturally, and graded a forgery `Attested`. Both key
   arguments are now a `VerifiedKid` newtype whose only mints are a completed verification
   (`VerifiedEvent::signer`, via a crate-private constructor) or a proof-carrying `event_log` column.
   **Careful with that second one:** `attester_key` alone is NOT proof — db/020's deferred arm stores a
   peer's token unverified, which is why SQL's R1 pairs the column with `cairn_attestation_vouched`. The
   old call is a **`compile_fail` doctest** (plus a positive companion, since rustdoc on stable accepts
   any compile error). `authority_lockstep.rs` no longer hand-supplies the attester (its own stated
   weakness): it reads `event_log.attester_key`, on the vouched path.
4. **The review of this branch found the SQL fix over-claiming in exactly the way it was correcting**, and
   four things came out of it. (a) The claim is now *sanctioned, not only*: the column is a copy of a
   clear body field, `signed_bytes` must stay granted, so `cairn_body(signed_bytes) -> 'safety'` returns
   the uncoarsened value to the same role (**#424**) — and the runtime role is a `cairn_node` member, a
   role db/049 never narrows (**#425**). db/049, ADR-0063's erratum, `safety_read_grants.rs` and ROADMAP
   all say so now. (b) **`SET search_path = public` does not exclude `pg_temp`** — Postgres searches the
   temp schema FIRST for relation names — so the two functions this slice made `SECURITY DEFINER` could be
   blinded by any caller creating a temp `event_log`, returning **zero rows** for a chart carrying a real
   warning, straight into `main.rs`'s "no safety signals on file" reassurance. Fixed here (`, pg_temp`
   last, pinned by test and by `proconfig` assertions); **every other definer in the repo still has it**
   (**#426**). (c) The column grant broke **whole-row** `event_log` readers (a `f(el)` composite needs
   SELECT on every column) — db/034's two medication-thread functions, reached from a `cairn_agent`-granted
   view; both became definers. (d) The narrowing is not continuous: db/005 re-grants the table on every
   replay and db/049 re-narrows ~44 files later, each file its own transaction (**#427**).
   Also: `VerifiedKid`'s mint-site allowlist is unpinned (**#428**), and rustdoc on stable ignores
   `compile_fail` error codes, so the negative doctest now has a positive companion.

**2026-08-15 — Slice 68: claim authority at the apply door** (closes
[#380](https://github.com/cairn-ehr/cairn-ehr/issues/380), discharges
[#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 2, gives
[#245](https://github.com/cairn-ehr/cairn-ehr/issues/245) its first SQL counterpart; **ADR-0064**, spec v0.66,
`SCHEMA_GENERATION` unchanged at 49 — no new migration). Full reasoning is ADR-0064's nine decisions. Four
things to carry:

1. **One predicate, one site.** `cairn_claim_authority(claim, target) → 'attested' | 'self' | 'unverified'`
   (db/005) is consulted at exactly one clause in `cairn_sensitivity_standing` (db/048), so display
   coarsening, safety-rung emission and the CLI path all inherit it with no per-consumer change — the
   anti-drift answer to #404's lesson that hand-maintained mirror pairs diverge.
2. **Gates effect, never admission, only in the withholding direction.** A claim below the bar still
   lands, converges and is re-assertable; it just does not lower a grade. No door refusal, so no fork
   (the [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342) trap); a raise is never impeded.
3. **Flag what cannot self-heal; view what can** is now a stated rule, not two precedents. The withdrawal
   worklist (`inert` / `stranger-attested`) is a VIEW because authority improves as targets replicate; the
   new `safety_overclaim_flag` — #405 part 2's fix, at the LOCAL door only — is a LEDGER because a
   published byte can never improve. **Neither has a shipped reader** (#388) — see ⇒ NEXT.
4. **Computing the verdict at read cuts both ways.** Both routes resolve through `actor_current`, which
   excludes a revoked actor, so revoking someone *after* their withdrawal landed silently re-raises the
   grade — safe in direction, undecided whether it is *right*
   ([#409](https://github.com/cairn-ehr/cairn-ehr/issues/409)). The Rust↔SQL authority mapping is
   separately known to diverge on two shapes, tracked as
   [#408](https://github.com/cairn-ehr/cairn-ehr/issues/408).

**The PR #410 review landed a second fix wave (2026-08-16).** Six review agents plus mutation testing
against a live PG18; **7 of 11 production-code mutations survived a green suite**, which is the review in
one statistic. Four defects were real and fixed there. What still generalises:

- **R2's self-identity equality was completely unpinned** — replacing `c.actor_id = t.actor_id` with
  `TRUE` left the suite green and reopened #380 in full, because every un-attested fixture used the
  *device* as both asserter and withdrawer, so R2 died on `kind = 'human'` and never reached the equality.
  Pinning it needs two DISTINCT human actors — hence `enroll_human_with_role` (`enroll_human` twice
  collides: same pinned set, same `actor_id`, refused enrollment).
- **`EXCEPTION WHEN OTHERS` does not catch a statement timeout** — PostgreSQL's `OTHERS` excludes
  `query_canceled` (57014) and `assert_failure`, so a blanket handler let a timeout abort `submit_event`
  and refuse the medication assert: the incident ADR-0063 decision 8 exists to prevent, reproduced by the
  block written to prevent it. **The one protection-stripping comparison was also fail-OPEN**
  (`<> 'unverified'` → `IN ('attested','self')`).
- **Comments asserting guarantees the code does not provide were the largest single class** — and #405
  part 1 / #412 (2026-08-16) were two more of exactly that. The pattern is stubborn enough that the #405
  *fix* re-committed it ("ONLY IS NOW ENFORCED"), and its own review caught that; treat a comment claiming
  a floor as unverified until someone has tried the bypass. ADR-0064's six wrong line citations are
  [#417](https://github.com/cairn-ehr/cairn-ehr/issues/417) (ADRs are immutable, and a resolve-in-range
  check does not catch the shift class that actually bites).

Filed, not fixed (**#412 was fixed 2026-08-16**): **#413** (KeyId/ActorId conflation, the root cause of
#408) · **#414** (the overclaim ledger's completeness rests on a `RAISE WARNING` nothing consumes — an
empty ledger is indistinguishable from a broken one) · **#415** (`stranger-attested` measures the SIGNER,
so it will fire on routine care — every shipped clinical verb is node-signed) · **#416** (a sealed
withdrawal is inert and invisible) · **#418** (constraining the verdict domain needs a DROP CASCADE
decision) · **#419** (coverage gaps: R1's conjuncts, the worklist's two untested arms) · **#420**
(`search_path` / PUBLIC revocation — note the new `SECURITY DEFINER` read functions widen it) · **#421**
(the worklist omits the accountable actor) · **#422** (no CHECK on the overclaim ledger's relation).

**2026-08-14 — Slice 67: the §5.9 safety projection, part B** (closes
[#375](https://github.com/cairn-ehr/cairn-ehr/issues/375), discharges
[#294](https://github.com/cairn-ehr/cairn-ehr/issues/294); **ADR-0063**, spec v0.65,
`SCHEMA_GENERATION` 48→49). Full reasoning is ADR-0063's eight decisions. Three things to carry:

1. **The seal boundary is the coarsening boundary.** Precise `{class, severity}` travels sealed with the
   body; a grade-chosen **rung** rides the envelope in the clear, so *coarsen-but-survive* after a
   crypto-shred is structural — the signal rides the append-only `event_log` row a shred never touches.
2. **Two coarsenings, load-bearing for DIFFERENT reasons.** Emission binds a peer's raw-SQL client; read
   answers a peer that legitimately emitted a finer rung (the grade is node-relative). **Read coarsening
   is a rendering choice, not a floor** — db/049 section 8 (2026-08-16) made the raw `SELECT safety` a
   privilege refusal for `cairn_agent`, but the value is still recoverable from `signed_bytes` and
   `cairn_node` keeps the table grant (#405 part 1 — narrowed, not closed; #424/#425; part 2, the
   emission-side rung-vs-grade check, was Slice 68/ADR-0064).
3. **`safety_class_map` ships EMPTY** — Cairn ships the lookup, never the drug knowledge; the seam drugref
   plugs into.

**The PR #403 review landed a fix wave, including [#404](https://github.com/cairn-ehr/cairn-ehr/issues/404):**
`cairn_prospective_sensitivity`'s thread arm had diverged from db/048, and because its two arms were
exhaustive `p_thread` was inert — a thread-scoped grade coarsened chart-wide and emission disagreed with
read on the same node. Fixed; the tripwire, the overload hazard and remaining open items are in ADR-0063
and git.

**2026-08-11 — Slice 66: custody follows admission** (closes
[#231](https://github.com/cairn-ehr/cairn-ehr/issues/231); **ADR-0052 §4 deferred**, erratum E1). The
unwrap-cert kid is now pinned to `trust_peer` (db/007); before it, any self-signed cert reaching the serve
port obtained read-custody of every non-shredded sealed body. **Withhold the key, never the bytes** — an
unadmitted puller still receives the events; refusing would fork the event set. **Repair is TWO steps**,
`pull --full` then `cairn_reproject()` (the sweep restores custody, the chart stays empty until
reprojected). Same day, PR #390: cargo-deny's v2 `unsound = "none"` default let an unsound advisory pass
in silence — `unsound = "all"` is now set in **both** `deny.toml` trees, with
[#389](https://github.com/cairn-ehr/cairn-ehr/issues/389) ignored with a reason and an expiry. Detail for
both in ROADMAP.

**2026-08-10 — Slice 65: the §5.9 sensitivity stream, part A** (#232 part A; **ADR-0062**, spec v0.64,
`SCHEMA_GENERATION` 47→48). Full reasoning is ADR-0062. Two things still worth carrying:

1. **Unknown ranks MAX, inverting db/040's `ELSE 0`** — there rank 0 withholds *reject power* (safe);
   here it would withhold *protection*. **Do not "fix" it into consistency.** Absence still ranks 0.
2. **The effective grade is node-relative.** A node with less custody deliberately computes a *higher*
   grade; gaining custody can lower a displayed grade. Any cross-node equality test needs *given equal
   custody* — sharpened by ADR-0064 §4's finding that it also needs equal actor-registry state.

**2026-08-09 — the loop's second unattended run + this doc prune.** `/techdebt-loop --max-issues 3`
merged three PRs (ROADMAP "Interlude — 08-09"): **#169** (destructive `db/tests` mirrors now refuse any
database not marked `cairn_scratch_database`), **#227** (the A3 HLC merge extracted into one guarded
`cairn_node_hlc_merge`), **#228** (malformed hex in a node-door payload now fails legibly with `P0001`).
Lesson: **`P0001` is a contract with the pull loop** — `cairn-sync` treats it as deliberate (skip,
re-offer) and anything else as transient (freeze the cursor), so a bare `decode()` inside a door stalls
sync from that peer permanently. [#370](https://github.com/cairn-ehr/cairn-ehr/issues/370) is the same
defect on the clinical plane and is still open.

**2026-08-08 — Slice 64: closing the funnel's bypass** (#345; `SCHEMA_GENERATION` 46→47). The first
event carrying a `patient_id` must be that chart's registration, refused at `submit_event`; the remote
apply door stays lenient **by design**. **Retiring `patient.created` was the load-bearing half** —
otherwise the rule reads *"…unless…"*, and an "unless" in a safety floor is where the next defect lives;
and when you add a rule to a shared file, re-check every subset that loads it (`cairn-sync` carried
db/005 but not db/045). **Deliberately NOT done:** the rule never reaches a patient named in a *payload*;
`patient.amended`/`note.added` survive unfloored ([#364](https://github.com/cairn-ehr/cairn-ehr/issues/364),
[#365](https://github.com/cairn-ehr/cairn-ehr/issues/365)).

**2026-08-05 — Slice 63: the funnel itself** (ADR-0061, spec v0.63). The one thing to carry into future
registration work: **the attestation NAMES the displayed candidates, it does not count them** — *was the
duplicate on screen when the clerk clicked create?* has opposite fixes for yes (fix the UI) and no (fix
the comparator), and `N = 3` cannot separate them. Follow-ons #346–#357, #359–#362 are in ROADMAP.

**2026-08-02/03 — Slices 61+62: the med-list node tier and window.** Full narrative in ROADMAP; three
lessons that generalise: **(1) a displayed row is a GROUP; an attestation is a THREAD** (ADR-0047
collapses reconciled duplicates into one line, ADR-0049 attests per thread — nearly every defect in that
build lived on the seam); **(2) a unit-tested safety control can still be defeated by the surface that
calls it** — the 15-minute idle re-lock never fired because a shared accessor counted every poll as
activity, with every `SessionKey` unit test passing, so **test the path the product actually calls**;
**(3) a compensating control outside CI is not a control** — `cairn-gui` is a separate cargo workspace
`cargo test --workspace` never covered; the `gui` CI job now does, ⚠️ still not REQUIRED (see ⇒ NEXT).

> [!IMPORTANT]
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md):
> *partial validity — a defect on one line never invalidates another.*** Read before any
> composite-clinical-object work: *the clinician gives an order and expects it to be carried out; it may
> be cancelled only by somebody taking ownership and giving a rationale*, hence **the system may fail to
> record an order, but it may never cancel one.** Binds orders/administration harder than sign-off; hold
> onto decision 2 (partial completion must be reported, never implied) and decision 7 (check the
> transaction boundaries).

**Three repo conventions these runs learned the hard way:**
- **Guard before connect.** DB-gated tests take `db::test_serial_guard(&base)` *before*
  `connect_and_load_schema`. Every existing suite does this in execution order.
- **UUIDs bind as text.** `cairn-node` does not enable tokio-postgres's `with-uuid-1`, so a `Uuid`
  parameter has no `ToSql`. Bind `&uuid.to_string()` and cast in SQL: `$1::text::uuid`.
- **A second human actor needs a distinguishing determinant.** `actor_id` content-addresses the *pinned
  determinant set*, so enrolling two clinicians as `{"role":"clinician"}` collides into one actor and is
  refused (P0001, ADR-0044/[#152](https://github.com/cairn-ehr/cairn-ehr/issues/152)). Add e.g.
  `"handle":"dr-b"`. The floor working as designed.

**Earlier sessions — condensed.** ROADMAP carries the per-slice detail (Slices 13–60, both tech-debt-loop
"Interlude" entries, every still-open issue enumerated). Two lessons from Slice 60: **a refusal that
persists nothing is a refusal you cannot audit**, and **when a call site cannot make a distinction, check
whether an intermediate layer threw it away** (`apply_signed` flattened `postgres::Error` to a `String`,
discarding the SQLSTATE separating a deliberate refusal from a transient fault). The arc, 2026-06-25 →
08-01: demographics + matcher · identity/John-Doe/medication build-out · the five-priority review course
→ ADR-0051–0058 · ADR-0059 + medication 6a/6b · the ADR-0056 admit-uninterpreted floor · floor determinism
(#75) · tech-debt-loop launch and its first nine unattended PRs.

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in
[`scratch/ui-sketches/easygp-consult-screen-inventory.md`](../scratch/ui-sketches/easygp-consult-screen-inventory.md)
and `easygp-editing-area-inventory.md` (source screenshots git-ignored under
`docs/untracked_for_brainstorming/` — real photos, **never commit or publish**). Headline: easyGP's six
editing-area invariants ≅ Cairn's event envelope near line-for-line. Awaiting graduation into the shell
spec: ten GUI principles, a GP-manifest seed, eleven principle-4 prior-art exhibits. **Open:** co-author
questions in the editing-area note §7; results-inbox screenshots pending — three-zone vs two-pane rides
on them, **don't improvise it**. **Scope:** the easyGP co-author may lead GP-facing GUI design, HH
designs ED & ward; the role-manifest layer is the seam (ADR-0021).

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
- **`docs/ROADMAP.md`** — the foundation build order (wire core → in-DB floor → sync → identity →
  security → federation → blobs → native API), *below* the policy/GUI line, plus the per-slice build
  narrative. Disposable scaffolding like this file; the spec/ADRs win on any disagreement.
- **`docs/spikes/`** — build-prep records (*what we tried, on what, what we learned*). Not spec, not ADR.
  0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor — C1–C5 ✓ →
  ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓); 0004 (iced reference UI — FAIL on a11y → Tauri 2).
- **`docs/case-studies/`** — clinical case-mining record. 0001 (2026-07-11): 16 Australian GP-software
  failure modes, all absorbed, **0 new architecture**, three action items (see Open threads).
- **`docs/ecosystem/`** — evals: 0001 (kastellan/localmail plugins), 0003 (reference-data sourcing).
  **`docs/principles/`** — mission/governance; root **`README.md`** repeats the founding principles.
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
  `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace).
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** — built 2026-06-21, the first implementation of
  [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md):
  `cairn-node` (Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS pinned to the trust set, set-union
  `node_event` sync) + the `db/007` doors with a deny-all admission gate; genesis-stable `node_id`.
  **Every honest gap declared at build time is CLOSED**, including all four ADR-0026 durability slices
  A–D — only optional escrow *rungs* (Shamir/QR/TPM) remain. The `localstate` read/apply **seams** are
  where the clinical tier plugs DEKs/drafts/config.
- **Dual-identifier discipline** — ADR-0031: the canonical plane (UUIDv7 + multihash) is the *only*
  identifier on the wire/in signed bodies; the projection plane may intern to node-local `bigint`
  surrogates (`db/008` + the leakage guard). The load-bearing guarantee is the typed signed plane.
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so plain `cargo test --workspace`
  is reliable. Connection strings and the DB-slice runner are under Open threads → Test env.
- **Tech-debt loop** — `/techdebt-loop` triages issues into `loop:*` labels and drives `/techdebt-next`
  one fresh headless session per issue until the ready backlog is dry (spec:
  `docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`). Auto-merge **ENABLED**; **works
  unattended** (12 PRs across two runs); currently **stopped** by maintainer decision. Cold-start ladder:
  `--dry-run`, `--max-issues 1` watched, then unbounded. Live gaps: **#326** (the worker's CI-wait idiom
  is dead in this harness), **#312** (triage never re-checks `loop:ready`), **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts C/D** ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) — A, B and the authority
  floor all shipped (Slices 65/67/68); **C is unblocked**, its open decision is the dial question (⇒ NEXT).
  Related: **#235** (shred authorization policy hooks), **#236** (FTS/RAG must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059 fully implemented 2026-07-28). **Next
  candidates:** the **drugref term→anchor lookup** (§9 advisory tier — the thing that actually closes the
  coded↔uncoded duplicate case; needs a cross-service connection-model decision first, and the source
  guard keeping the trusted surface drugref-free must stay passing); fuzzy/automatic reconciliation + a
  Tier-A drug dictionary (brand↔generic/DDI beyond the exact-anchor case); structured sig/frequency
  (lands with prescriptions); correcting a dose event's *effective date* on the statement-level `started`.
  **Cross-cutting debt:** [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (**cross-thread
  correction *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design
  decision**); [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory onto the
  medication/dose/reconciliation projections; [#176](https://github.com/cairn-ehr/cairn-ehr/issues/176)
  (oversize-guard remote-apply test). Spine to reuse: `db/031`–`db/033`, `db/041`, `db/042` +
  `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine to reuse: `db/010`–`db/030` +
  `cairn-event::demographics`; everything in the "Built so far" paragraph is DONE).
  **Next (B3 measurement-driven):** a **large hand-crafted gold set** to re-run the learner for
  authoritative magnitudes (slice 24's learner is a PoC on small/synthetic data); locale comparator packs;
  the hub-tier duplicate sweep; proposal retraction; richer §7.5 matcher-actor determinants. **Next
  identity:** C5+ `reattribute` (**waits on a clinical-note surface**; a pending+disputed Doe already
  reads `'under-review'`, severity-max, so the slice-D forcing rule stands down while a dispute is open);
  the §5.12 push-alert. Smaller deferred items live in the issues:
  [#79](https://github.com/cairn-ehr/cairn-ehr/issues/79) (B2 minors),
  [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168) (entity→role-actor 1:many),
  [#287](https://github.com/cairn-ehr/cairn-ehr/issues/287) (sweep re-scores standing orphans); the
  unfiled ones (in code comments) are enumerated in ROADMAP's "Still open from slices 36–56".
- **Test env:** Rust DB-gated + matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532
  user=hherb dbname=cairn_test"` (PG18+cairn_pgx); the multi-node convergence suites additionally need
  `CAIRN_TEST_PG2`/`PG3` pointing at `cairn_test2`/`cairn_test3` on the same cluster (without them those
  tests **self-skip and cargo counts them as passed**, so a workspace count alone cannot distinguish skip
  from pass — CI sets all three since #199). Matcher integration: `cd matcher &&
  CAIRN_TEST_PG=… uv run --extra pipeline pytest`; the pure matcher suite is dependency-free (`cd matcher
  && uv run pytest`) — uv, never venv/pip. The `db/tests/*.sql` **mirrors run only via
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

## Parked (don't re-litigate without new reason)

- **Stewarding legal entity & jurisdiction** (German Stiftung/Verein, US 501(c)(3), or an umbrella) —
  deferred until momentum/funding geography is clearer. **Formal trademark / wordmark registration** —
  principle recorded (stewardship doc); legal instrument deferred.

---

## Working context

**CLAUDE.md carries this in full and is loaded every session** — the working conventions, the twelve
founding principles (the first four being the lens for every design choice), and the §9
defect-blast-radius language rule. Not restated here; canonical docs win.

- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr`
  org; `cairn-ehr.org`+`.com`; PyPI/crates.io/npm `@cairn-ehr` placeholders).
