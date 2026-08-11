# HANDOVER — Cairn

## ⇒ NEXT

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems, and
only PART A is built.** Slice 65 (2026-08-10, merged as
[PR #384](https://github.com/cairn-ehr/cairn-ehr/pull/384),
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md), spec v0.64,
`SCHEMA_GENERATION` 48) shipped the **sensitivity stream**: graded, append-only confidentiality assertions
over an event / a medication thread / a whole chart, whose effective grade is the **max** over standing
assertions on all three. **It enforces nothing** — it computes and reports a grade. Read ADR-0062 before
touching any of the remaining parts; do not re-derive its ten decisions.

Three parts remain, and their order is forced:

- **Part B — safety-projection emission** ([#375](https://github.com/cairn-ehr/cairn-ehr/issues/375)):
  de-identified class + severity, coarsened by the grade. **Buildable now.** Carries
  [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) — the class must be captured **pre-seal and
  carried**, never re-derived by the reader (ADR-0059 decision 4).
- **Part C — sequester / custody narrowing** ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)).
  **UNBLOCKED 2026-08-11** — [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) landed (Slice 66),
  so `serve` now pins the unwrap cert's `kid` to `trust_peer` and custody genuinely follows admission.
  Part C is the next thing to build in this thread and nothing blocks it.
- **Part D — break-glass** ([#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)): audited key-*use*,
  partition-honest. Blocked on C.

Slice 65's own follow-ons: [#374](https://github.com/cairn-ehr/cairn-ehr/issues/374) (thread resolution
resolves only a thread's *current head* — see **erratum E4**, the limitation is real but narrower than
the ADR first stated), [#378](https://github.com/cairn-ehr/cairn-ehr/issues/378) (the withdrawal
rationale is clear text forever and replicates — the UI must warn at entry today),
[#379](https://github.com/cairn-ehr/cairn-ehr/issues/379) (the grade in the legibility twin), and the
sensitivity gesture kinds added to #360's `ui_gesture_timing_kind_ck` widening. The comprehensive review
added four more: [#385](https://github.com/cairn-ehr/cairn-ehr/issues/385) (index `content_address` on
the five medication projections — `cairn_event_thread` is currently the #336 shape it cites),
[#386](https://github.com/cairn-ehr/cairn-ehr/issues/386) (the cairn-sync subset test loads db/048 but
never *drives* it, so the late-binding guard is untested at runtime),
[#387](https://github.com/cairn-ehr/cairn-ehr/issues/387) (type-design tightenings — a `Provenance` enum,
ladder constants, a sum type for the report's correlated `Option` pair) and
[#388](https://github.com/cairn-ehr/cairn-ehr/issues/388) (the operator surface is blind to withdrawals,
deferred grades, and custody-less charts). Four more came out of the same review and are the ones to read
before part B: **[#380](https://github.com/cairn-ehr/cairn-ehr/issues/380) — nothing on the wire controls
a protection-REMOVING act**; any enrolled actor on any peer can strip any grade, un-attested, because the
withdrawal type is `targets_other_author = FALSE` (no ADR-0043 owner-gate), an omitted responsibility
contributor asks for no attestation, and the ceremony is local-door only. It follows correctly from
ADR-0062 decision 7 and is recoverable by re-assertion, but it is the **only** lowering path in Cairn with
no cross-door control at all — `loop:needs-human`, three directions weighed on the issue (detect on a
worklist / grade the withdrawal as low-confidence / accept-and-bound).
Then [#383](https://github.com/cairn-ehr/cairn-ehr/issues/383) (`chart_sensitivity` builds its breakdown
from `medication_statement`, so a **custody-thin node** — the offline tier this project exists for —
prints "no medication threads" while honouring standing thread-scoped grades; derive the trailing count
from `cairn_sensitivity_standing`, which is readable without custody),
[#382](https://github.com/cairn-ehr/cairn-ehr/issues/382) (most `cairn_check_*` floor fns lack
`REVOKE EXECUTE … FROM PUBLIC`) and [#381](https://github.com/cairn-ehr/cairn-ehr/issues/381) (the
db/tests/048 mirror covers two of the sensitivity cases; F1's cross-chart pin is Rust-only).

> [!IMPORTANT]
> **Slice 65's review round changed the floor's behaviour in four ways. Read ADR-0062's errata E4/E5/E6
> before building part B or C on it** — the ceremony is now keyed on blast radius rather than
> `subject_kind = 'patient'`, the mis-target rule covers all three kinds, a `category` key is refused at
> the local door, and a sealed assertion coarsens instead of vanishing.
>
> The one that is a **code trap rather than a design change**, and so is repeated here: the catch-all arm
> reports `subject_kind = 'coarsened'`, so a consumer must test "did anything win" with
> **`content_address IS NOT NULL`, never `subject_kind <> 'none'`** — `none` is a legal open-vocabulary
> value and collided with the sentinel.

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
2. **The accessibility pass** — a live screen-reader run (VoiceOver) through the runbook's eight checks,
   keyboard-only: `cargo run -p cairn-gui-tauri -- --mock --patient
   00000000-0000-0000-0000-000000000001`. The fixture chart deliberately carries a cross-patient line and
   an invisible group so the ADR-0060 warnings are exercised. Automating the DOM assertions is
   [#332](https://github.com/cairn-ehr/cairn-ehr/issues/332) and needs a JS-toolchain decision this slice
   deliberately did not take (plain JS, no npm, no bundler).
3. **Make the new `gui` CI job a REQUIRED status check.** "clippy + cargo test (cairn-gui)" (PR #343)
   gates the reference-UI workspace and its JS/Rust drift guard, but only a repo admin can add it to
   main's branch protection; until then it can go red without blocking a merge, which is most of the
   value gone. (Match the job name exactly, per the warning in `rust.yml`.)

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
   surface drugref-free and must stay passing.
3. **The node/actor plane's two divergences** — one slice, one prerequisite. `db/007` still fail-closes on
   an unmappable type ([#301](https://github.com/cairn-ehr/cairn-ehr/issues/301), slice 58) and still
   skips-and-advances a verifiable refusal where the clinical plane now pens
   ([#268](https://github.com/cairn-ehr/cairn-ehr/issues/268), slice 60). **Neither is a symmetric fix:**
   `node_event` is type-shaped, so #301 needs a carried-not-interpreted row shape plus an audit of every
   trust projection; #268 needs the door to tell "not-for-me trust-graph deny-all" (routine scoping) from
   "genuinely refused history", or the pen fills with steady-state traffic. Both stay `loop:blocked`.
4. **[#370](https://github.com/cairn-ehr/cairn-ehr/issues/370) — the clinical plane's copy of the #228
   defect.** A malformed `digest_hex` in an attachment reference raises in the `22` class, which
   `cairn-sync` reads as a transient fault and freezes the pull cursor on. Same shape as PR #371, one
   plane over: an availability defect wearing a legibility defect's clothes.

**Standing gate:** whole-project review cycles repeat periodically, and there will be **no release for
clinical use before repeated review cycles pass cleanly.** The last full pass ran 2026-07-15 (in-DB floor,
Rust workspace, spec/ADR corpus, matcher, cross-cutting seams —
[report](code_reviews/2026-07-15-whole-project-architecture-review.md), findings #187–#217), **fully
closed**. A runnable clinical surface exists that has never been through one — include it next.

**The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
session holds the main repo — safe to re-run when the repo is free (`tail -f ~/.cairn-loop/run.log`).
**Never start it alongside a human session**: on 08-02 a stray loop `cargo test --workspace` ran ~5 h
against the same cluster, stretching that session's suites from ~3 min to ~90 min (they contend on one
cargo lock and one `test_serial_guard` advisory lock).

---

**Session date:** 2026-08-11 (doc currency; `main` clean, no open PRs) · **Spec/ADRs:** v0.64 (through
**ADR-0062**, *the sensitivity stream and the inverted unknown*) · **`SCHEMA_GENERATION`:** 48 (`db/048`)
· **Phase:** architecture complete (every original §11
question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list
window.

**Built so far** (full detail in ROADMAP + the ADR log + git):
**demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex/
gender-identity · §4.3 address; karyotype resolved as a distinct field, ADR-0037, no code yet) ·
the **§5.2 advisory Python matcher** (in-DB veto floor · scoring core · veto-gated pipeline/blocking ·
the B3 eval harness, compound blocking keys, synthetic volume generator, supervised Fellegi–Sunter
weight-learning) ·
the **§5.7 identity core C1–C5** (linkage · human-accepted apply seam · auto-apply band · dispute ·
identify · repudiate + the known-alias pool — the confirmed/unconfirmed/under-review contract is
COMPLETE; C5+ `reattribute` waits on a clinical-note surface) ·
the **§5.4 John-Doe subsystem** (slices A–D + finishers + photo/text evidence + the `enroll-human`
ceremony CLI; still open: the §5.12 push-alert) ·
the **§5.3/§5.8 search-before-create funnel** (ADR-0061 — `identity.registration.asserted` + its db/045
floor and retained-set projection, the advisory db/046 candidate search, the pure `cairn-patient-search`
read model, `patient-search`/`patient-register`, John Doe re-expressed onto the same act) and its
**precedence rule** (#345 — `cairn_patient_has_events` + db/005 step 8b; `patient.created` retired in
db/047, which also handed registration the `patient_chart` chart-birth projection) ·
the **first clinical-content stream `clinical.medication`, slices 1–6b** (assert/cease + the E1
reconciliation flag · bitemporal dose timeline · cross-thread reconciliation ADR-0047 · the attestation
responsibility overlay ADR-0049 · per-field dose correction ADR-0050 · the inline `substance.coding`
drug-identity shape and the two coding-overlay verbs, ADR-0059) + the **twin-check registry** (ADR-0048) ·
the **contributor-role vocabulary floor** (ADR-0051) ·
**born-sealed clinical bodies** (ADR-0052 — plus **custody follows admission** since #231/Slice 66:
`serve` pins the unwrap-cert `kid` to `trust_peer`, so this is confidentiality-capable, not merely an
erasability substrate) ·
**per-write human authorship** (ADR-0053 — grading half-live until #245) ·
the **§5.9 sensitivity stream, part A** (ADR-0062 — graded append-only confidentiality assertions + the
effective-grade read model + three CLI verbs; **enforces nothing yet**, B/C/D open) ·
the **L3 reference UI** — `cairn-gui/`, a standalone workspace deliberately detached from the root one
via its `exclude` entry so no GUI stack can enter `cairn-node`'s dependency tree; the dependency
direction is one-way, GUI → crates, forever. The iced shell (PR #174) FAILED the accessibility bar
(spike 0004) and was **retired 2026-08-03**; what runs today is **`cairn-gui-tauri`** — a Tauri 2 window
on one patient's medication chart (whole-list sign-off, per-row cease, plain-JS semantic-HTML webview,
no npm). The pane/routing/freshness state machine survived the retirement and is tested but not wired ·
the **med-list node tier** (Cairn's first clinical READ path + whole-list sign-off + two CLI verbs) ·
**generic reprojection** (ADR-0057 — one registered apply fn per projection + one dispatcher) ·
the **ADR-0056 admit-uninterpreted floor** (the clinical door admits an unclassifiable type; power granted
only by re-adjudication) + **the residual refusal contract on the clinical plane** (a deliberate floor
refusal is penned verbatim, pinned, auto-released on repair, and a frozen watermark fails loud).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node,
Postgres-on-Android).

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative; this section keeps only what a *next* session needs.

**2026-08-11 — Slice 66: custody follows admission** (closes
[#231](https://github.com/cairn-ehr/cairn-ehr/issues/231); no new ADR — implements the hardening
**ADR-0052 §4 deferred**, recorded there as **erratum E1**; no schema change). `cairn-sync serve`
verified a puller's unwrap-key certificate against its own signature and self-consistency only, so the
ADR's *"re-wraps for any **admitted** peer"* had its qualifier unenforced: **any self-signed cert
reaching the serve port obtained read-custody of every non-shredded sealed body.** The kid is now pinned
to `trust_peer` (db/007) — the third consumer of the one trust set, after the mTLS cert-pin verifier and
`refresh_trust_set`. Four things to carry:

1. **Withhold the key, never the bytes.** An unadmitted puller still receives the events and its pull
   still succeeds: sealed ciphertext is harmless without a DEK, and refusing it would fork the event set
   and wedge replication for no confidentiality gain. Same degradation an absent cert already took.
2. **The repair is `pull --full`, NOT "pull again"** — a durable operational fact, not a slice detail.
   Withheld custody *is* repairable (`apply_remote_event` has no duplicate early-return and its custody
   insert is `ON CONFLICT DO NOTHING`, so a re-offer carrying a DEK fills the gap), but an incremental
   pull asks only for `seq > cursor`, which is already past the custody-less events. A shred is the one
   irreparable case, deliberately. Caught by asking whether the remedy I had printed could actually be
   run — the Slice 61 lesson turned on my own text.
3. **Six sites in `clinical_pull.rs` had to gain a peering ceremony, and that IS the finding** — every
   one had been obtaining custody with no admission at all. The new sibling test asserts the security
   case directly: an unadmitted puller replicates the events and gains `(0, 0)` custody.
4. **`trust_peer` reads empty on a provisioned-but-unpeered node AND on an uninitialised one**, because
   it filters on a `local_node` subquery that is NULL in the second. The first draft collapsed both into
   one line telling the operator to run `cairn-node init` **on an already-initialised node** — a remedy
   that cannot work. Found by reading the serve log of a *passing* test, not by an assertion; the lookup
   now reads `local_node` too, and a unit test pins the two apart.
5. **`cairn-sync`'s SCHEMA subset deliberately still excludes db/007**, so this is a SOFT dependency:
   `42P01` maps to its own arm, withholds, and names the missing provisioning. Adding db/007 to the
   subset would collide with db/001's `hlc_state` — that is #284's decision, not this slice's.

**2026-08-11 — the supply-chain gate could not see unsound advisories** (PR #390). cargo-deny's advisories
config v2 defaults `unsound = "none"`, so an advisory carrying `informational = "unsound"` passed the gate
in silence while Dependabot alerted on the same GHSA — a green check that could not see undefined
behaviour in a dependency of a medical record. `unsound = "all"` is now set in **both** `deny.toml` trees.
One finding falls out, ignored with a written reason and an expiry:
[#389](https://github.com/cairn-ehr/cairn-ehr/issues/389) — glib 0.18.5 `VariantStrIter` (RUSTSEC-2024-0429,
MODERATE, crash-only, Linux-only, no Cairn call site); patched is ≥ 0.20.0, unreachable until Tauri's Linux
backend moves to gtk-rs 0.20+.

**2026-08-10 — Slice 65: the §5.9 sensitivity stream, part A** (#232 part A; **ADR-0062**, spec v0.64,
`SCHEMA_GENERATION` 47→48 for `db/048`). Full reasoning is ADR-0062 — four things a next session needs:

1. **Unknown ranks MAX, inverting db/040's `ELSE 0`.** There an unrecognised value ranking 0 withholds
   *reject power* (safe); here it would withhold *protection*, so an older node reads a peer's newer grade
   as "not sensitive" and renders a confidential body in the clear. **Do not "fix" it into consistency.**
   Absence still ranks 0 — no assertion is `routine`; only an unrecognised grade *value* coarsens.
2. **The ceremony is exactly three rules and is LOCAL-door only** — a chart-wide raise must name its own
   chart and carry a rationale; a withdrawal needs a bound human author — and the remote door is *tested*
   to admit all three: a check at apply forks the event set (#342), and for a *raise* the refusal would
   itself be a disclosure. **The withdrawal's non-empty rationale is NOT ceremony**: it is a structural
   floor dispatched through `cairn_event_twin` at **both** doors. Structural checks judge the *shape of
   the claim* and are safe everywhere; ceremony checks judge *who authored it* and must stay local.
3. **The effective grade is node-relative.** Thread membership needs custody, so a node with less custody
   deliberately computes a *higher* grade; **gaining custody can lower a displayed grade.** Any cross-node
   equality test is valid only *given equal custody* — the qualifier is in the test's name for that reason.
4. **It enforces nothing, on purpose.** A projection-layer filter with no custody narrowing beneath it is
   theatre a raw-SQL client walks past. Enforcement is part C, which #231/Slice 66 has now unblocked.

**2026-08-09 — the loop's second unattended run + this doc prune.** `/techdebt-loop --max-issues 3`
merged three PRs and closed three issues (ROADMAP "Interlude — 08-09"): **#169** (PR #367 — `db/tests`
mirrors now refuse any database not carrying a `cairn_scratch_database` marker), **#227** (PR #369 — the
A3 HLC merge extracted from five pasted copies into one guarded `cairn_node_hlc_merge` in `db/001`),
**#228** (PR #371 — malformed hex in a node-door payload fails legibly *and* with `P0001`). Two lessons
outlive them:

1. **`P0001` is now a contract with the pull loop, not an accident of how a raise is written.**
   `cairn-sync` classifies a refusal on a *verified* event by SQLSTATE: `P0001` → deliberate, skip and
   re-offer; anything else → assumed transient, **freeze the cursor**. So a bare `decode()` (class `22`)
   inside a door is not a cosmetic slip — it stalls sync from that peer **permanently**, while logging
   "transient … (not skipped past)", i.e. telling the operator to wait for something that will never
   clear. [#370](https://github.com/cairn-ehr/cairn-ehr/issues/370) is the same defect on the clinical
   plane and is still open.
2. **An allow-list, not a name deny-list.** The destructive `db/tests` mirrors now refuse any database not
   *explicitly declared disposable* — the dangerous target is precisely the one nobody thought to name.

**2026-08-08 — Slice 64: closing the funnel's bypass** (#345; no new ADR — ADR-0061 decision 3 had
already decided the rule and deferred only its enforcement; `SCHEMA_GENERATION` 46→47). The first event
carrying a `patient_id` must be that chart's registration, refused at `submit_event`; the remote apply
door stays lenient **by design** (set-union sync has no ordering; a fail-closed remote door would wedge
replication), and that asymmetry is tested, not merely commented. Two things to carry:

1. **Retiring `patient.created` was the load-bearing half, not a tidy-up** — otherwise the rule would read
   *"…must be a registration, **unless**…"*, and an "unless" in a safety floor is where the next defect
   lives. db/047 is the precedent every future type retirement copies (projection rows deleted **before**
   the classification row).
2. **When you add a rule to a shared file, re-check every subset that loads it.** `cairn-sync`'s SCHEMA
   subset carried db/005 but not db/045 — from the moment db/005 gained an opinion about registration,
   **a door carrying a rule it could not satisfy**. (Slice 66 is the same seam, other direction.)

**Deliberately NOT done (Slice 64):** the rule is envelope-scoped and never reaches a patient named in a
*payload*; `patient.amended` / `note.added` survive as unfloored walking-skeleton projection vehicles
([#364](https://github.com/cairn-ehr/cairn-ehr/issues/364), [#365](https://github.com/cairn-ehr/cairn-ehr/issues/365)).

**2026-08-05 — Slice 63: the funnel itself** (ADR-0061, spec v0.63). The one thing to carry into future
registration work: **the attestation NAMES the displayed candidates, it does not count them** — *was the
duplicate on screen when the clerk clicked create?* has opposite fixes for yes (fix the UI) and no (fix
the comparator), and `N = 3` cannot separate them. Follow-ons #346–#357, #359–#362 are in ROADMAP.

**2026-08-02/03 — Slices 61+62: the med-list node tier and window.** Full narrative in ROADMAP; four
lessons that generalise beyond the slice:

1. **A displayed row is a GROUP; an attestation is a THREAD.** ADR-0047 collapses reconciled duplicates
   into one line; ADR-0049 attests per thread. Nearly every defect this build surfaced lived on that
   seam. The shared crate exists so the node and the UI cannot answer *"what is about to be signed?"*
   differently.
2. **A safety refusal is only as good as the escape hatch it names.** All three cross-patient warnings
   told the operator to run `medication-separate` — a verb taking two THREAD ids — while printing only a
   GROUP id, so the remedy was reachable only by raw SQL. **Check every error message that names a fix:
   can the reader actually run it from what you just printed?**
3. **A unit-tested safety control can still be defeated by the surface that calls it.** The 15-minute
   idle re-lock never fired: the window polls `lock_state` every 10 s, the poll shared an accessor with
   sign-off, and the accessor counted every call as activity — so the window reset its own idle clock
   forever and a held signing key outlived any absence. **Every `SessionKey` unit test passed.** Fixed by
   splitting `key_status` (reads) from `live_key` (uses); expiry now also consults the **wall clock**
   (`Instant` does not advance while a laptop sleeps). **Test the path the product actually calls.**
4. **A compensating control outside CI is not a control.** `cairn-gui` is a separate cargo workspace, so
   `cargo test --workspace` had never covered a line of it. A **`gui` CI job** now runs fmt/clippy/test/
   deny there — ⚠️ still **not a REQUIRED status check** (see ⇒ NEXT).

> [!IMPORTANT]
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md):
> *partial validity — a defect on one line never invalidates another.*** A corollary of paper-parity that
> earned its own ADR because it was violated *by a design that had already accepted paper-parity*, with
> no test catching it. **Read the ADR** before any composite-clinical-object work — the framing to lead
> with is *the clinician gives an order and expects it to be carried out; it may be cancelled only by
> somebody **taking ownership** and **giving a rationale***, hence **the system may fail to record an
> order, but it may never cancel one.** It will bind the orders/administration surface far harder than
> sign-off; hold onto decision 2 (**partial completion must be reported, never implied**) and decision 7
> (**check the transaction boundaries**).

**Three repo conventions these runs learned the hard way:**
- **Guard before connect.** DB-gated tests take `db::test_serial_guard(&base)` *before*
  `connect_and_load_schema`. Every existing suite does this in execution order.
- **UUIDs bind as text.** `cairn-node` does not enable tokio-postgres's `with-uuid-1`, so a `Uuid`
  parameter has no `ToSql`. Bind `&uuid.to_string()` and cast in SQL: `$1::text::uuid`.
- **A second human actor needs a distinguishing determinant.** `actor_id` content-addresses the *pinned
  determinant set*, so enrolling two clinicians as `{"role":"clinician"}` collides into one actor and is
  refused (P0001, ADR-0044/[#152](https://github.com/cairn-ehr/cairn-ehr/issues/152)). Add e.g.
  `"handle":"dr-b"`. The floor working as designed.

**Earlier sessions — condensed.** ROADMAP carries the per-slice detail (Slices 13–35, 36–56 and 57–60
each condensed there, plus both tech-debt-loop "Interlude" entries, with every still-open issue
enumerated). Two lessons from Slice 60 are worth keeping here: **a refusal that persists nothing is a
refusal you cannot audit**, and **when a call site cannot make a distinction, check whether an
intermediate layer threw it away** (`apply_signed` flattened `postgres::Error` to a `String`, discarding
the SQLSTATE that separates a deliberate `P0001` refusal from a transient fault). The arc, 2026-06-25 →
08-01: demographics slices 1–5 + the §5.2 matcher pieces · the identity/John-Doe/medication build-out and
CI catch-up · the five-priority review course P1–P5 and the Priority-6 design queue → ADR-0051 through
ADR-0058 · the matcher follow-on batches · ADR-0059 and medication slices 6a/6b · the ADR-0056
admit-uninterpreted floor, its review round and the residual refusal contract · floor determinism (#75),
tech-debt-loop launch readiness and its first nine unattended PRs.

**GUI/L3 design threads (2026-07-16/18, design-only).** Full detail in
[`scratch/ui-sketches/easygp-consult-screen-inventory.md`](../scratch/ui-sketches/easygp-consult-screen-inventory.md)
and [`easygp-editing-area-inventory.md`](../scratch/ui-sketches/easygp-editing-area-inventory.md) (source
screenshots git-ignored under `docs/untracked_for_brainstorming/` — real photos, **never commit or
publish**). Headline: easyGP's six editing-area invariants ≅ Cairn's event envelope near line-for-line —
external validation that the envelope is the right user-facing grammar. Awaiting graduation into the shell
spec: **ten GUI principles**, a **GP-manifest seed**, eleven principle-4 prior-art exhibits (the med-list
window already honours several — state ambient never modal, no confirmation dialogs). **Open:** co-author
questions in the editing-area note §7; results-inbox screenshots pending — the three-zone vs two-pane
question rides on them, **don't improvise it**. **Team/scope:** the easyGP co-author may return to lead
**GP-facing GUI design**; HH designs **ED & ward** once core infra is nailed down; the shell's
role-manifest layer is the seam (ADR-0021 working as intended).

**Status of this file:** Disposable working scaffolding, **not** a source of truth. Regenerate at the end
of each session, and **keep it under 500 lines** — its value is inversely proportional to its length
(#368). If it disagrees with the canonical docs, **the canonical docs win.** The *why* lives in the
immutable ADR log, the *what* in the spec; this file carries only what lives *between* them — current
build state, open threads, time-sensitive items.

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
- **`docs/ecosystem/`** — evals, neither spec nor ADR: 0001 (kastellan/localmail plugins), 0003
  (reference-data sourcing, fed ADR-0025). **`docs/principles/`** — mission/governance
  (`GOVERNANCE.md` + `STEWARDSHIP-OF-THE-NAME.md`); root **`README.md`** repeats the founding principles.
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
  `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace).
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** — built 2026-06-21 ([PR #28](https://github.com/cairn-ehr/cairn-ehr/pull/28)),
  the first implementation of [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md):
  `cairn-node` (Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS pinned to the trust set, set-union
  `node_event` sync) + the `db/007` doors with a deny-all admission gate; genesis-stable `node_id`.
  **Every honest gap declared at build time is CLOSED** (detail in git + ROADMAP Phases 5/6), including
  all four [ADR-0026](spec/decisions/0026-node-durability-and-disaster-recovery.md) durability slices A–D
  — only optional escrow *rungs* (Shamir M-of-N / QR / TPM) remain, upward options, not blockers. The
  `localstate` DB read/apply **seams** are where the clinical tier plugs DEKs/drafts/config.
- **Dual-identifier discipline** — ADR-0031: the canonical plane (UUIDv7 + multihash) is the *only*
  identifier on the wire/in signed bodies; the projection plane may intern to node-local `bigint`
  surrogates (`db/008` + the leakage guard). The load-bearing guarantee is the typed signed plane.
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so plain `cargo test --workspace`
  is reliable. Connection strings and the DB-slice runner are under Open threads → Test env.
- **Tech-debt loop** — `/techdebt-loop` triages issues into `loop:*` labels and drives `/techdebt-next`
  one fresh headless session per issue until the ready backlog is dry (spec:
  `docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`). **Auto-merge is ENABLED**
  (2026-07-31; probeable via `autoMergeRequest` on a recently merged PR). **It works unattended**: 12 PRs
  merged across two runs (07-31 → 08-01, and 08-09). Currently **stopped** by maintainer decision. Cold
  start ladder: `--dry-run`, then `--max-issues 1` watched, then unbounded. Known live gaps:
  [#326](https://github.com/cairn-ehr/cairn-ehr/issues/326) (the worker's CI-wait idiom is dead in this
  harness — cycles complete by tight polling), [#312](https://github.com/cairn-ehr/cairn-ehr/issues/312)
  (triage never re-checks `loop:ready`), [#322](https://github.com/cairn-ehr/cairn-ehr/issues/322).

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts B/C/D** ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) — **part A shipped as
  Slice 65**, and **C is no longer blocked**: #231 landed as Slice 66, so the unwrap-cert kid is pinned
  to the node-plane trust set and custody follows admission. See ⇒ NEXT.
  Related: [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294)
  (carry the drug class, don't re-derive it), [#235](https://github.com/cairn-ehr/cairn-ehr/issues/235)
  (shred authorization policy hooks), [#236](https://github.com/cairn-ehr/cairn-ehr/issues/236) (FTS/RAG
  must build on `event_clear` only).
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
  the hub-tier aggressive duplicate sweep; proposal retraction; richer §7.5 matcher-actor determinants
  (served-model digest). **Next identity:** C5+ `reattribute` (§5.5 event-granular strike-through of
  *clinical documentation* — **waits on a clinical-note surface**; a pending+disputed Doe already reads
  `'under-review'`, severity-max, so the slice-D forcing rule stands down while a dispute is open); the
  §5.12 "prior history now available" push-alert. Karyotype is resolved as a distinct field (ADR-0037) —
  no code yet. Smaller deferred items live in the issues:
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
  from Case 0001: **① re-affirmation-without-change currency** ([#163](https://github.com/cairn-ehr/cairn-ehr/issues/163)
  — the envelope records asserted-since vs confirmed-current-as-of; every `patient_*` projection collapses
  both into one overwrite-on-reaffirm winner); **② open-loop/obligation** (order/recall/referral with no
  closing ack) may warrant a named projection, surfaced by salience not a modal; **③ impossible-vs-uncertain**
  constraint rule for the in-DB floor (reject only the physically/type-impossible, advisorily flag the
  merely improbable).
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already
  advanced across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#9-bet-b--results-raspberry-pi-5--8-gb-2026-06-25--pass-with-two-honest-caveats)):
  **PASS twice**, most recently the clean 2026-07-07 re-run on PG 18.4 + a PCIe NVMe HAT with both
  caveats resolved — B1 p95 **3.99 ms @ 2,004,000 events** (13× under budget), B2 p95 4.5 ms/374-note
  chart, ~1,515 B/event on disk; B4 confirms ADR-0015's BLAKE3 blob-digest default (~4× SHA-256 on
  Cortex-A76). **Remaining:** fold the now un-caveated B4 number into the ADR-0015 follow-up to drop
  "provisional" from the blob-digest line. Also [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272)
  (re-run the reproject bench on the Pi rig).
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
