# HANDOVER — Cairn

## ⇒ NEXT

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems. A, B,
the cross-cutting authority floor and the operator surface over all three are built; ⇒ C is next.** Read
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) and
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) before touching the rest; **do not
re-derive their decisions.**

- **Part A** (Slice 65, ADR-0062) — graded append-only assertions over an event / a thread / a whole
  chart; effective grade is the **max** over all three. Computes and reports only.
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
- **The operator surface** (Slice 69, 2026-08-18; closes
  [#388](https://github.com/cairn-ehr/cairn-ehr/issues/388),
  [#383](https://github.com/cairn-ehr/cairn-ehr/issues/383),
  [#421](https://github.com/cairn-ehr/cairn-ehr/issues/421)) — `patient-sensitivity <chart>` reports
  ineffective withdrawals (reason + rationale + accountable actor), deferred `sensitivity.%` events, the
  standing assertions a custody-thin node cannot anchor, and safety overclaims. **ADR-0064's §1.2 budget
  is MET** (erratum E1) and pinned by a test.
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

**Two §5.9 leaks were closed 2026-08-16** (#412, #405 — both CLOSED). Carry two facts.
**`REVOKE SELECT (column)` is inert while a table-level grant stands**, so `cairn_agent` holds an
explicit 23-column grant on `event_log` omitting `safety`, and **adding a column to `event_log` now
requires granting it in db/049 section 8** (fail-closed; `safety_read_grants.rs` names the missing
column). And — the correction that matters most — **that grant is cost-raising, not a floor**: the
column copies a *clear* field of the signed body, so `cairn_body(signed_bytes) -> 'safety'` still returns
it uncoarsened, and the runtime role is a `cairn_node` member which keeps the table grant
([#425](https://github.com/cairn-ehr/cairn-ehr/issues/425),
[#427](https://github.com/cairn-ehr/cairn-ehr/issues/427)). **Never cite db/049 section 8 as a
confidentiality boundary**; ADR-0063 decision 2 (emission-time coarsening) is the one that binds. The
open design question — *should a node attempt a confidentiality boundary below the envelope at all?* —
is **[#432](https://github.com/cairn-ehr/cairn-ehr/issues/432)**, split out when #424 was closed with
its Wants unanswered.

Slice 65's own follow-ons: **#374** (thread resolution resolves only a thread's *current head* — erratum
E4 narrows it), **#378** (the withdrawal rationale is clear text forever and replicates — the UI must warn
at entry today), **#379** (the grade in the twin), **#381** (db/tests/048 mirror parity), **#382**
(`REVOKE EXECUTE` on the `cairn_check_*` family), **#385** (index `content_address` on the five
medication projections) and **#387** (type design — it touches the report structs Slice 69 just grew,
and was deliberately kept out of that slice). **#386 is half-closed** — db/049's subset test *drives*
it; db/048's still does not. **#383 and #388 closed 2026-08-18.**

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
   measured — median 222 ms, in `results/2026-08-03-node-tier-write-cost.md`, which says **PARTIAL** in
   its title for this reason. Slice 63 owes BOTH halves for registration (≤ 5 s to find an existing
   chart, ≤ 20 s to register a new one); the node-tier write-cost half is
   [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360) (nothing is wired; db/044's `gesture_kind`
   CHECK refuses a registration row until widened).
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`. The fixture
   chart deliberately carries a cross-patient line and an invisible group so the ADR-0060 warnings are
   exercised. Automating the DOM assertions is **#332** (needs a JS-toolchain decision: plain JS, no npm).
3. **Make the `gui` CI job a REQUIRED status check.** "clippy + cargo test (cairn-gui)" (PR #343) gates
   the reference-UI workspace and its JS/Rust drift guard, but only a repo admin can add it to main's
   branch protection; until then it can go red without blocking a merge. Match the job name exactly.

**If a measurement falls outside its budget, that is the finding — file an issue; do not adjust the
budget to match.**

**The other build candidates** (any can be picked up next; nothing blocks a choice):

1. **The registration/search UI slice** — the picker is the wrong-chart affordance paper has and the
   med-list window does not. **Constraint from Slice 63:** the picker must **open** a chart, never
   *retarget* an open window — retargeting re-creates the §5.8 item 4 / §5.11 windowing misfile that
   possession semantics exist to prevent. Also wires the kept-but-unwired pane/routing/freshness machine.
2. **The drugref term→anchor lookup** — the §9 *advisory* tier, and what actually closes the
   **coded↔uncoded** duplicate case ADR-0059 decision 5 leaves open. Needs a design decision first: the
   cross-service connection model. The slice-6a/6b source guard keeps the trusted surface drugref-free
   and must stay passing. **Slice 67 gave it a second consumer:** `safety_class_map` is the empty seam
   drugref would populate, and today no node has any class knowledge at all.
3. **The node/actor plane's two divergences** — `db/007` fail-closes on an unmappable type (**#301**) and
   skips-and-advances a verifiable refusal where the clinical plane now pens (**#268**). **Neither is a
   symmetric fix**, and both are `loop:blocked`.
4. **[#370](https://github.com/cairn-ehr/cairn-ehr/issues/370)** — the clinical plane's copy of #228: a
   malformed `digest_hex` raises in the `22` class, which `cairn-sync` reads as transient and freezes the
   pull cursor on. An availability defect wearing a legibility defect's clothes.
5. **[#387](https://github.com/cairn-ehr/cairn-ehr/issues/387)** — type design over the §5.9 report
   structs Slice 69 just grew (a `Provenance` enum, ladder constants, a sum type for the correlated
   `Option` pair). Deliberately kept out of Slice 69 to keep both diffs reviewable.

**Standing gate:** whole-project review cycles repeat periodically, and there will be **no release for
clinical use before repeated review cycles pass cleanly.** Last full pass 2026-07-15
([report](code_reviews/2026-07-15-whole-project-architecture-review.md), findings #187–#217), **fully
closed**. A runnable clinical surface exists that has never been through one — include it next.

**The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
session holds the main repo (`tail -f ~/.cairn-loop/run.log`). **Never start it alongside a human
session**: they contend on one cargo lock and one `test_serial_guard` advisory lock (a stray loop once
stretched a session's suites from ~3 min to ~90 min).

> [!TIP]
> **A live IDE contends the same way, and it is not obvious.** rust-analyzer's `cargo check
> --workspace --all-targets` holds the shared `target/` lock, so a narrow `cargo test` blocks before it
> compiles, then times out. Fix is a scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the IDE. **The old
> "recreate cairn_test/2/3 after an `event_log` column add" note is OBSOLETE** — since #296 the suites
> build `event_log` rows by name via `jsonb_populate_record`, so the stale-column-order failure is
> structurally closed.

---

**Session date:** 2026-08-18 (the §5.9 operator surface) · **Spec/ADRs:** v0.66 (through **ADR-0064**,
*admit the claim, withhold the power*; ADR-0063 gained erratum E1, **ADR-0064 gained erratum E1**) · **`SCHEMA_GENERATION`:** 49 (`db/049`) ·
**Phase:** architecture complete (every original §11 question closed); **first production clinical
surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** (full detail in ROADMAP + the ADR log + git):

- **Demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex /
  gender-identity · §4.3 address; karyotype resolved as a distinct field, ADR-0037, no code yet).
- **The §5.2 advisory Python matcher** — in-DB veto floor, scoring core, veto-gated pipeline/blocking,
  the B3 eval harness, compound blocking keys, volume generator, Fellegi–Sunter weight-learning.
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
  `kid` to `trust_peer` · **per-write human authorship** (ADR-0053 — grading half-live until #245).
- **The §5.9 stream, COMPLETE through its read surface** — parts A/B, the authority floor and the
  operator surface (see ⇒ NEXT). **Enforces nothing beyond display/emission.**
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

**2026-08-18 — Slice 69: the §5.9 operator surface** (closes
[#388](https://github.com/cairn-ehr/cairn-ehr/issues/388),
[#383](https://github.com/cairn-ehr/cairn-ehr/issues/383),
[#421](https://github.com/cairn-ehr/cairn-ehr/issues/421); **ADR-0064 gains erratum E1**; no new ADR, no
new migration file, `SCHEMA_GENERATION` stays 49). Three slices had shipped a §5.9 mechanism and no way
to look at it; `patient-sensitivity <chart>` is now the one query that tells the whole truth. Four things
to carry:

1. **NAME, NEVER COUNT.** #388 part 3 and #383 *both* asked for a count of standing assertions. The
   slice deliberately diverges from both: ADR-0061 settled the shape — *"3 standing assertions, 0 threads
   shown"* cannot separate **custody-blind** from **genuinely empty**, which is the one question the line
   exists to answer, and a named row carries the `content_address` `sensitivity-withdraw --withdraws`
   consumes. If a later reader "simplifies" this back to a count, that is a regression.
2. **A chart-scoped definer, not a table grant.** `event_deferred` is granted to `cairn_node`, NOT
   `cairn_agent` — a direct read works only via the membership #425 flags as unreliable. So db/043 gained
   `cairn_patient_deferred_sensitivity(uuid)`, and its test pins **why it exists** (asserting
   `cairn_agent` genuinely lacks `SELECT` on `event_deferred`) so nobody later mistakes the definer for
   ceremony and replaces it with a grant.
3. **The report declares what it cannot contain.** ADR-0064's permanently-invisible cross-chart
   withdrawal and #414's unconsumed `RAISE WARNING` are both printed in the footer, and both disclaimers
   are asserted by tests over **empty** lists — the case where silence is most convincing and most wrong.
4. **Two Rust traps worth the next session's time.** A type named only inside `#[cfg(test)]` compiles
   clean under `cargo test --lib` and fails the integration build under `-D warnings`; `--lib` alone does
   not catch that class. And `search_path_pg_temp.rs` compares `rows.len() >= PINNED_TODAY`, deliberately
   — a 26th definer needs **no** number moved (verified, not assumed).


**2026-08-16 (later) — the pinned `search_path` that pinned nothing** (closes
[#426](https://github.com/cairn-ehr/cairn-ehr/issues/426); 21 function headers gained `, pg_temp`).
**It was live data loss at both owner-rights write doors, not hygiene:** `SET search_path = public` does
not exclude the session temp schema, so with a decoy `event_log` in place `submit_event` and
`apply_remote_event` each **RETURNED SUCCESS while the owner-privileged INSERT landed in the caller's
temp table** — demonstrated as `cairn_agent`, a role with no write privilege on `event_log` at all. The
guard is over `pg_proc`, not a name list: every pinned path must **deny the temp schema the first look**
(not merely "end in `pg_temp`" — that naive rule waves through `SET search_path = pg_temp`), and every
`SECURITY DEFINER` must pin one at all. `pg_temp` last closes RELATION and DATA-TYPE lookup outright and
has nothing to do with function or operator lookup (PostgreSQL never searches temp for those).
**Residual:** it only wins a name `public` actually HAS — an unqualified relation *absent* from `public`
still falls through, reaching `to_regclass()` in `cairn_check_projection_registry_fn`. Deliberately NOT
done: `REVOKE TEMPORARY … FROM PUBLIC` (policy at the wrong layer, and it would disarm the tests proving
the fix). Still open: **[#430](https://github.com/cairn-ehr/cairn-ehr/issues/430)** (the unpinned
invoker-rights surface — ~100 functions, ~68 reachable by `cairn_agent`; no RLS policy or `CHECK`
consults one, so no escalation path today, but `cairn_patient_has_events` is safe only by *inheriting*
`submit_event`'s path) and **[#431](https://github.com/cairn-ehr/cairn-ehr/issues/431)**
(`cairn_execute_shred` has catalogue-only coverage; a diverted shred would report an erasure that never
happened).

**2026-08-16 — two live §5.9 leaks, one per plane** (closes #412 and #405; **ADR-0063 gained erratum
E1**). Both were **a guarantee asserted in a comment that the code did not provide** — and the review of
the fix found it had reproduced that exact shape in its own prose. Three things to carry:

1. **A column-level `REVOKE` cannot narrow a table-level `GRANT`.** Postgres tracks them separately, and
   a table grant covers every column added later. db/049 section 8 therefore drops `cairn_agent` to an
   explicit 23-column grant and the two read functions became `SECURITY DEFINER` — **the two halves are
   one fix**, mutation-checked both ways. The fail-closed cost is deliberate: a future `event_log` column
   is unreadable until granted. Residual replay window: **#427**.
2. **A parameter name is not a security property.** `classify_authorship_confidence(&body.contributors,
   &body.signer_key_id, None)` compiled, read naturally, and graded a forgery `Attested`. Both key
   arguments are now a `VerifiedKid` newtype minted only by a completed verification or a proof-carrying
   `event_log` column. **Careful:** `attester_key` alone is NOT proof — db/020's deferred arm stores a
   peer's token unverified, which is why SQL's R1 pairs it with `cairn_attestation_vouched`. Mint-site
   allowlist unpinned: **#428**.
3. Whole-row `event_log` readers broke under the column grant (a `f(el)` composite needs SELECT on every
   column), so db/034's two medication-thread functions became definers too.

**2026-08-15 — Slice 68: claim authority at the apply door** (closes #380, discharges #405 part 2, gives
#245 its first SQL counterpart; **ADR-0064**, spec v0.66). Full reasoning is ADR-0064's nine decisions.
Four things to carry:

1. **One predicate, one site.** `cairn_claim_authority(claim, target) → 'attested' | 'self' |
   'unverified'` (db/005) is consulted at exactly one clause in `cairn_sensitivity_standing` (db/048), so
   display coarsening, safety-rung emission and the CLI path all inherit it with no per-consumer change —
   the anti-drift answer to #404's lesson that hand-maintained mirror pairs diverge.
2. **Gates effect, never admission, only in the withholding direction.** A claim below the bar still
   lands, converges and is re-assertable; it just does not lower a grade. No door refusal, so no fork
   (the **#342** trap); a raise is never impeded.
3. **Flag what cannot self-heal; view what can** is now a stated rule. The withdrawal worklist is a VIEW
   because authority improves as targets replicate; `safety_overclaim_flag` is a LEDGER because a
   published byte can never improve. **Both gained readers in Slice 69.**
4. **Computing the verdict at read cuts both ways.** Both routes resolve through `actor_current`, which
   excludes a revoked actor, so revoking someone *after* their withdrawal landed silently re-raises the
   grade — safe in direction, undecided whether it is *right* (**#409**). The Rust↔SQL authority mapping
   separately diverges on two shapes (**#408**, root cause **#413**).

**The PR #410 review landed a second fix wave (2026-08-16).** Six review agents plus mutation testing
against a live PG18; **7 of 11 production-code mutations survived a green suite**, which is the review in
one statistic. What generalises:

- **R2's self-identity equality was completely unpinned** — replacing `c.actor_id = t.actor_id` with
  `TRUE` left the suite green and reopened #380 in full, because every un-attested fixture used the
  *device* as both asserter and withdrawer. Pinning it needs two DISTINCT human actors.
- **`EXCEPTION WHEN OTHERS` does not catch a statement timeout** — PostgreSQL's `OTHERS` excludes
  `query_canceled` (57014) and `assert_failure`, so a blanket handler let a timeout abort `submit_event`:
  the incident ADR-0063 decision 8 exists to prevent, reproduced by the block written to prevent it. The
  one protection-stripping comparison was also **fail-OPEN**.
- **Comments asserting guarantees the code does not provide were the largest single class** — and the
  #405 *fix* re-committed it, caught by its own review. Treat a comment claiming a floor as unverified
  until someone has tried the bypass.

Filed, not fixed: **#413** (KeyId/ActorId conflation, root cause of #408) · **#414** (the overclaim
ledger's completeness rests on a `RAISE WARNING` nothing consumes; now *declared* in the Slice 69 footer)
· **#415** (`stranger-attested` measures the SIGNER, so it will fire on routine care — every shipped
clinical verb is node-signed; **Slice 69 makes this visible, so expect noise**) · **#416** (a sealed
withdrawal is inert and invisible) · **#417** (six wrong ADR line citations; ADRs are immutable) ·
**#418** (constraining the verdict domain needs a DROP CASCADE decision) · **#419** (coverage gaps) ·
**#420** (`search_path`/PUBLIC — narrowed by #426; what remains is the inlining measurement on
`cairn_sensitivity_standing`) · **#422** (no CHECK on the overclaim ledger's relation) · **#409** ·
**#408**.

**2026-08-14 — Slice 67: the §5.9 safety projection, part B** (closes #375, discharges #294;
**ADR-0063**, spec v0.65, `SCHEMA_GENERATION` 48→49). **The seal boundary is the coarsening boundary:**
precise `{class, severity}` travels sealed with the body, a grade-chosen **rung** rides the envelope in
the clear, so *coarsen-but-survive* after a crypto-shred is structural — the signal rides the `event_log`
row a shred never touches. **Two coarsenings, load-bearing for DIFFERENT reasons:** emission binds a
peer's raw-SQL client; read answers a peer that legitimately emitted a finer rung (the grade is
node-relative), and **read coarsening is a rendering choice, not a floor**. **`safety_class_map` ships
EMPTY** — Cairn ships the lookup, never the drug knowledge; the seam drugref plugs into. Open: **#407**
(the sealed precise claim is read by nothing), **#406** (no supersession — a ceased drug warns forever),
**#401**, **#398**, **#397**, **#395**, **#394**, **#400** (render the warning in the UI and measure its
§1.2 budget), **#399**, **#402**. The PR #403 review also fixed **#404**:
`cairn_prospective_sensitivity`'s thread arm had diverged from db/048 and, because its two arms were
exhaustive, `p_thread` was inert — a thread-scoped grade coarsened chart-wide and emission disagreed with
read on the same node.

**2026-08-11 — Slice 66: custody follows admission** (closes #231; ADR-0052 §4 deferred, erratum E1).
The unwrap-cert kid is pinned to `trust_peer` (db/007); before it, any self-signed cert reaching the
serve port obtained read-custody of every non-shredded sealed body. **Withhold the key, never the
bytes** — an unadmitted puller still receives the events; refusing would fork the event set. **Repair is
TWO steps**, `pull --full` then `cairn_reproject()`. Same day (PR #390): cargo-deny's v2 `unsound =
"none"` default let an unsound advisory pass in silence — `unsound = "all"` is now set in **both**
`deny.toml` trees, with **#389** ignored with a reason and an expiry.

**2026-08-10 — Slice 65: the §5.9 sensitivity stream, part A** (#232 part A; **ADR-0062**, spec v0.64).
Two traps still worth carrying:

1. **Unknown ranks MAX, inverting db/040's `ELSE 0`** — there rank 0 withholds *reject power* (safe);
   here it would withhold *protection*. **Do not "fix" it into consistency.** Absence still ranks 0.
2. **The effective grade is node-relative.** A node with less custody deliberately computes a *higher*
   grade; gaining custody can lower a displayed grade. Any cross-node equality test needs *given equal
   custody* — and, per ADR-0064 §4, equal actor-registry state too.

**2026-08-05 — Slice 63: the funnel itself** (ADR-0061, spec v0.63). The one thing to carry into future
registration work: **the attestation NAMES the displayed candidates, it does not count them** — *was the
duplicate on screen when the clerk clicked create?* has opposite fixes for yes (fix the UI) and no (fix
the comparator), and `N = 3` cannot separate them. **Slice 69 applied the same rule to the standing-
assertion list.** Follow-ons #346–#357, #359–#362 are in ROADMAP; the §1.2 write-cost half is **#360**.

**2026-08-02/03 — Slices 61+62: the med-list node tier and window.** Three lessons that generalise:
**(1) a displayed row is a GROUP; an attestation is a THREAD** (ADR-0047 collapses reconciled duplicates
into one line, ADR-0049 attests per thread — nearly every defect in that build lived on the seam);
**(2) a unit-tested safety control can still be defeated by the surface that calls it** — the 15-minute
idle re-lock never fired because a shared accessor counted every poll as activity, with every
`SessionKey` unit test passing, so **test the path the product actually calls**; **(3) a compensating
control outside CI is not a control** — `cairn-gui` is a separate cargo workspace `cargo test
--workspace` never covered; the `gui` CI job now does, ⚠️ still not REQUIRED (see ⇒ NEXT).

> [!IMPORTANT]
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md):
> *partial validity — a defect on one line never invalidates another.*** Read before any
> composite-clinical-object work: *the clinician gives an order and expects it to be carried out; it may
> be cancelled only by somebody taking ownership and giving a rationale*, hence **the system may fail to
> record an order, but it may never cancel one.** Hold onto decision 2 (partial completion must be
> reported, never implied) and decision 7 (check the transaction boundaries).

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
tech-debt-loop "Interlude" entries, every still-open issue enumerated). Two lessons from Slice 60: **a
refusal that persists nothing is a refusal you cannot audit**, and **when a call site cannot make a
distinction, check whether an intermediate layer threw it away** (`apply_signed` flattened
`postgres::Error` to a `String`, discarding the SQLSTATE separating a deliberate refusal from a transient
fault). The arc, 2026-06-25 → 08-01: demographics + matcher · identity/John-Doe/medication build-out ·
the five-priority review course → ADR-0051–0058 · ADR-0059 + medication 6a/6b · the ADR-0056
admit-uninterpreted floor · floor determinism (#75) · tech-debt-loop launch and its first nine PRs.

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in
[`scratch/ui-sketches/easygp-consult-screen-inventory.md`](../scratch/ui-sketches/easygp-consult-screen-inventory.md)
and `easygp-editing-area-inventory.md` (source screenshots git-ignored under
`docs/untracked_for_brainstorming/` — real photos, **never commit or publish**). Headline: easyGP's six
editing-area invariants ≅ Cairn's event envelope near line-for-line. **Open:** co-author questions in the
editing-area note §7; results-inbox screenshots pending — three-zone vs two-pane rides on them, **don't
improvise it**. **Scope:** the easyGP co-author may lead GP-facing GUI design, HH designs ED & ward; the
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
  fresh headless session per issue until the ready backlog is dry. Auto-merge **ENABLED**; **works
  unattended** (12 PRs across two runs); currently **stopped** by maintainer decision. Cold-start ladder:
  `--dry-run`, `--max-issues 1` watched, then unbounded. Live gaps: **#326**, **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts C/D** ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) — A, B and the authority
  floor all shipped (Slices 65/67/68); **C is unblocked**, its open decision is the dial question (⇒ NEXT).
  Related: **#235** (shred authorization policy hooks), **#236** (FTS/RAG must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059 fully implemented 2026-07-28). **Next
  candidates:** the **drugref term→anchor lookup** (⇒ NEXT item 2); fuzzy/automatic reconciliation + a
  Tier-A drug dictionary (brand↔generic/DDI beyond the exact-anchor case); structured sig/frequency
  (lands with prescriptions); correcting a dose event's *effective date* on the statement-level `started`.
  **Cross-cutting debt:** [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (cross-thread
  correction *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision).
  Spine to reuse: `db/031`–`db/033`, `db/041`, `db/042` + `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine: `db/010`–`db/030` +
  `cairn-event::demographics`; everything under "Built so far" is DONE). **Next (B3
  measurement-driven):** a **large hand-crafted gold set** to re-run the learner for authoritative
  magnitudes (slice 24's learner is a PoC on synthetic data); locale comparator packs; the hub-tier
  duplicate sweep; proposal retraction; richer §7.5 matcher-actor determinants. **Next identity:** C5+
  `reattribute` (**waits on a clinical-note surface**); the §5.12 push-alert. Deferred:
  [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168) (entity→role-actor 1:many),
  [#287](https://github.com/cairn-ehr/cairn-ehr/issues/287) (sweep re-scores standing orphans); unfiled
  ones are enumerated in ROADMAP's "Still open from slices 36–56".
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
