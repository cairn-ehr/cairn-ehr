# Design — Human-attested clinical responsibility on medications (slice 4 of the `clinical.medication` surface)

**Date:** 2026-07-14 · **Branch:** `feat/medication-attestation-responsibility` (proposed) · **Status:**
design, pending implementation plan.

**Scope of change:** additive Rust (`cairn-event::medication::attestation`, a split
`cairn-node::medication` module + orchestrators + `--attest-as` on **every** existing verb + a new
`medication-attest` CLI) + one new DB migration (`db/034_medication_attestation.sql`: floor + a set-commitment
helper + attestation overlay/projection) + **[ADR-0049](../../spec/decisions/)** recording the
*commitment-based staleness* (currency-of-a-sign-off) precedent + a spec version bump (v0.49 → v0.50,
`index.md` only). Adds one verb to the medication surface opened by
[slice 1](2026-07-11-medication-recording-design.md),
extended by [slice 2](2026-07-12-medication-dose-overlay-design.md) and
[slice 3](2026-07-13-medication-reconciliation-resolution-design.md). **No new founding principle, no new
envelope field, no floor bypass, no wire change to existing med events** — this graduates the slices-1–3 §8
deferral ("human-attested clinical responsibility on a medication event") into product code, reusing the
settled attestation gate (db/005, ADR-0030), the compositional-authorship principle (ADR-0007, principle
10), and the append-only overlay + connected-component projection primitives.

---

## 1. What this is, and why now — the un-vouched clinical record

Every medication event authored today (slices 1–3: `asserted` / `-cessation` / `-dose-change` /
`-dose-correction` / `-reconciliation` / `-separation`) is **device-additive**: signed by the node key,
contributor `{"role":"recorded"}`, **no human takes responsibility for the clinical claim**. That is correct
for the *recording* act (the device faithfully records what it was told, offline-first) but it means the
medication list carries no accountability signal: a clinician cannot distinguish "a drug that a responsible
clinician has reviewed and vouches for" from "a drug the device recorded from some source, unvouched."

In a real ED/hospital this distinction is **the** medication-reconciliation workflow. A patient's list is
assembled from many sources (patient-reported, pharmacy import, prior notes, a John-Doe belongings scan). On
admission — and again at each transition of care — a clinician **reviews the list and takes responsibility
for it**, drug by drug. That sign-off is a first-class clinical act with medico-legal weight; the paper
equivalent is the signed reconciliation form. Cairn has no way to record it yet. This slice adds it.

**Principle 10 is the lens** ([ADR-0007](../../spec/decisions/0007-authorship-and-accountability.md)):
*authorship is compositional; accountability is separable.* Signature proves origin/integrity; **attestation
confers responsibility**. The two are genuinely separable here — the device records the fact, and a human
(possibly a *different* human, possibly *later*) vouches for it. So responsibility is modelled not by
changing who signs the medication event, but as a **separable attestation overlay** that references the
thread and carries the responsible human.

**Blast radius → substrate ([§9](../../spec/language-substrate.md)).** Accountability on the clinical record
is safety-critical (a forged or mis-attributed vouch is a medico-legal defect; a stale sign-off masquerading
as current can mask a dangerous change): pure Rust builders + an **in-DB floor** (the db/005 attestation gate
does the real enforcement) + a projection. The *display* of who-vouched-and-is-it-current is a read-side
projection. Structure mirrors slices 1–3 exactly.

---

## 2. Two workflows, one mechanism

The user asked for **both** author-time responsibility ("I vouch for this as I record it") and post-hoc
sign-off ("I reviewed this existing entry and take responsibility"). Because responsibility is a **separable
per-thread overlay**, one event type serves both:

- **Post-hoc** — author a `clinical.medication-attestation.asserted` referencing an existing
  `medication_id`, later, possibly by a different clinician than the recorder. This is the med-reconciliation
  sign-off.
- **Author-time** — record the medication event (device-additive, *unchanged*) **and** author its attestation
  in one atomic transaction (the `identify --link` two-event pattern). "I recorded this and I vouch for it."

Granularity is **per medication thread** (the user's choice): a clinician vouches for one drug's current
state at a time. A whole-list sign-off is then simply N thread attestations (composable; not a distinct
event). This keeps the target a single well-defined `medication_id` and the projection a per-drug
attested/unattested/stale badge.

---

## 3. Event vocabulary — one verb over a thread

| Event type | schema_version | mode | Body carries |
|---|---|---|---|
| `clinical.medication-attestation.asserted` | `clinical.medication-attestation/1` | **additive** | `medication_id`, `reviewed_commitment` (hex), `reviewed_count` (int), `basis?`, `note?` |

- **Additive**, `targets_other_author = FALSE`. An attestation adds accountability; it forecloses on nothing
  and touches no other author's content — so ADR-0043's cross-author owner-gate does **not** apply, and a
  clinician may vouch for a thread someone else recorded (exactly the med-rec case).
- **Responsibility-bearing.** The sole contributor is the vouching human:
  `[{"actor_id": human_kid, "role": "attested", "responsibility": "attested"}]`. The `responsibility` key is
  what trips the **existing** db/005 attestation gate (the `v_bears` path) — the event must go through the
  **3-arg `submit_event($1,$2,$3)`** door with a valid human attestation token bound to it, and the attester
  must be an enrolled `kind='human'` actor. This is the same gate the C2 identity link and the John-Doe
  `identify --link` use; **no floor special-case, no db/005 change.**
- **Honest-unknown** (principle 4): `basis`/`note` are inserted only when present, never serialized as null,
  so an added-later field never changes an existing event's content address (the demographics idiom).

### 3.1 The `reviewed_commitment` — how a sign-off pins what it vouched for

`reviewed_commitment` is a convergent commitment to the **set** of the thread's content events the clinician
reviewed, following the proven [`event_set_commitment`](../../crates/cairn-node/src/medium.rs) *recipe*: the
thread's content-event `content_address`es, **sorted** (byte order — collation-free), concatenated, and
hashed. Crucially it is **implemented as one SQL function** (`cairn_medication_thread_commitment`, §4.2) used
at both author and read time — never a Rust value compared against a SQL value — so the exact hash framing is
an internal detail (pgcrypto `digest(…, 'sha256')`, already a dependency, is fine; there is no Rust↔SQL
byte-identity requirement to satisfy and thus no drift risk). `reviewed_count` is the number of those events,
pinned purely as a twin/worklist legibility hint ("reviewed 4 entries"); it is **not** the staleness
authority.

"Thread content events" = the thread's own clinical content: `clinical.medication.asserted`,
`-cessation`, `-dose-change`, `-dose-correction` with the matching `medication_id`. **Reconciliation edges are
excluded** — they change the *group*, not the thread's content; group-membership changes are handled by the
group rollup (§5.3).

---

## 4. The floor — `db/034_medication_attestation.sql`

Additive migration; `db/031`–`db/033` **untouched**. Registers the type in **one additive
`cairn_event_twin_check` row** (the [ADR-0048](../../spec/decisions/0048-twin-check-registry-dispatch.md)
payoff — no dispatcher edit) + one `event_type_class` row, plus:

1. `cairn_check_medication_attestation(p_type text, b jsonb) RETURNS void` — the structural floor. Culture-
   neutral, **offline-first** (mirrors reconciliation — no check that the thread exists locally, since
   set-union sync may deliver the attestation before the thread): valid `medication_id` UUID + valid
   `patient_id` + a well-formed `reviewed_commitment` (non-empty, hex) + a non-negative integer
   `reviewed_count` + a non-empty authored twin (the HARD twin requirement). The **human-responsibility**
   enforcement is entirely the db/005 gate; this floor is purely structural.
2. `cairn_medication_thread_commitment(p_medication_id uuid) RETURNS bytea` — the **single source** of the
   commitment. Sorts the thread's content-event `content_address`es, concatenates, `digest(…, 'sha256')`
   (pgcrypto, already used in 6 migrations). Used at **both** ends — the orchestrator calls it at author time
   to compute the pin, the projection calls it to compute "current" — guaranteeing byte-identity with no
   Rust↔SQL drift. (Format detail: the function's output is the sole reference on both sides, so it need not
   match Rust's multihash framing — SQL is the only computor.)
3. `medication_attestation` — append-only projection table, one row per attestation event
   `(event_id PK, medication_id, patient_id, attester_kid, reviewed_commitment, reviewed_count,
   hlc_wall, hlc_counter, content_address, updated_at)`, populated by an **AFTER-INSERT** trigger on
   `event_log` gated `WHEN (NEW.event_type = 'clinical.medication-attestation.asserted')` (door-agnostic —
   fires for both the local and the remote-apply door; mirrors db/033's `_apply` trigger).

---

## 5. Projections (all **separate** views — the current-list view is never widened)

**Hard constraint.** `patient_medication_current` is re-issued by `CREATE OR REPLACE VIEW` with an identical
column set in db/031, db/032, **and** db/033 on every reconnect (`connect_and_load_schema` re-runs all
migrations). A db/034 that *widened* it would make db/033's narrower re-creation fail with *"cannot drop
columns from view"* on the next connect. So attestation ships as **separate views** a consumer joins to —
`patient_medication_current` is left exactly as db/033 leaves it.

1. `medication_thread_head` — per `medication_id`, the max convergent position
   `(hlc_wall, hlc_counter, content_address COLLATE "C")` over the thread's **content events only**
   (`assert / cessation / dose-change / dose-correction`). Used only for a cheap legibility "last changed"
   read; the *commitment* is the staleness authority.
2. `medication_thread_attestation` — per `medication_id`: the **latest** attestation (by its own convergent
   position), exposing `attester_kid`, the "attested as-of" HLC, `reviewed_count`, and:

   > **`stale` (boolean) = `cairn_medication_thread_commitment(medication_id) IS DISTINCT FROM reviewed_commitment`**

   A thread with no attestation is simply **absent** from this view (unattested). A thread whose content
   events aren't present locally (orphan attestation) is also absent — it renders nothing until the thread's
   events arrive (mirrors db/031's orphan-cessation), then computes staleness correctly.
3. `medication_group_attestation` — per `group_id` (via db/033's `medication_thread_group`): the **conservative
   rollup** — a group is *attested & current* iff **every member thread** (active or ceased — `medication_thread_group`
   carries no status filter) has a non-stale attestation; any member unattested-or-stale ⇒ the group is not
   current. Singletons (`group_id = medication_id`) reduce trivially to their thread's state.

   **Post-implementation correction:** as shipped, this is genuinely every member regardless of active/ceased
   status, not "every active member" — the conservative direction (a ceased duplicate still gates the group).

A current-list consumer (a future GUI, or a `medication-list` CLI) **LEFT JOINs**
`patient_medication_current × medication_group_attestation` on `group_id`. Replay-safe.

### 5.1 Staleness semantics (the design's load-bearing decision)

Stale is defined by **set commitment**, not causal position. Because the thread's content-event set is
**append-only (grow-only, never substituted)** and the commitment binds the exact set:

- a later change (any HLC position, including a **lower-HLC event that syncs in after the sign-off** — the
  GP-updated-this-morning-hasn't-synced case) → the set changes → the commitment changes → **stale**;
- a divergent set on another node → commitment differs → **stale** (errs toward re-review — the safe
  direction);
- nothing changed → identical set → identical commitment → **current**.

A **head-position pin would silently mis-classify** the lower-HLC late arrival as "reviewed" (it is causally
*below* the pinned head), and could show a same-count-but-divergent set on another node as "current." The
commitment closes both. **Residual, honestly:** you cannot review an event that exists on no reachable node
yet — that is reviewing the future, not a gap; the §5.13 background re-review sweep remains defense-in-depth.

---

## 6. Rust — builders, orchestrators, CLI

### 6.1 `cairn-event::medication::attestation` (new pure module)
Mirrors `reconciliation.rs`. `MedicationAttestation { medication_id, reviewed_commitment, reviewed_count,
basis?, note? }` → `medication_attestation_body(&a)` (inserts optional fields only when present) +
`render_medication_attestation_twin(&a)` (always non-empty; e.g. *"Reviewed & vouched for thread …abcd
(4 entries) — admission reconciliation"*). Re-exported from `medication/mod.rs`.

### 6.2 `cairn-node::medication::attestation` (new module in the split dir)
- `build_attestation_body(event_id, medication_id, patient, reviewed_commitment, reviewed_count, basis?,
  note?, human_kid, hlc) -> EventBody` — pure; emits the responsibility-bearing contributor + authored twin.
- `attest_thread_in_tx(tx, human_sk, human_kid, patient, medication_id, hlc, basis?, note?) -> Uuid` — the
  shared primitive: computes the commitment via `cairn_medication_thread_commitment` over the thread as
  visible **inside the caller's transaction** (so it includes any content event the caller just submitted),
  builds the body, signs with the **human** key, mints the token via `sign_attestation`, submits through the
  **3-arg** door. Returns the attestation event id. Every author path below is a thin wrapper over this.
- `attest_medication_thread(client, human_sk, human_kid, node_origin, patient, medication_id, basis?, note?)`
  — the **post-hoc** standalone orchestrator: mints an HLC, opens a one-statement txn, calls
  `attest_thread_in_tx`, commits. Bails with a clear message if the thread is not visible locally (the
  commitment fn returns null / no content events — "you review what you can see," like
  `resolve_correction_target`). The db/005 gate is the real enforcement.
- **Author-time, all verbs.** Each existing verb gains an `attest: Option<AttestParams<'_>>` path
  (`AttestParams { human_sk, human_kid, basis?, note? }`). When present, the orchestrator runs the verb's
  own submit **and** the attestation in ONE transaction: mint the verb HLC + one attestation HLC per affected
  thread up front (self-committing, like `identify_patient`) → open txn → `submit_event(verb_body)` →
  `attest_thread_in_tx(...)` for each affected thread → commit. A rejected attestation rolls the verb back
  (never recorded-but-unvouched-when-a-vouch-was-intended).
  - single-thread verbs (`assert` / `cease` / `dose-change` / `dose-correction`) attest their one
    `medication_id`;
  - the **pair** verbs (`reconcile` / `separate`) attest **both** subject threads (two attestation events in
    the same txn) — taking responsibility for both threads' current content as part of declaring them the
    same / different drug. (The reconcile/separate event itself stays device-additive; reconciliation edges
    are excluded from thread content, §3.1, so the two commitments are over each thread's own unchanged
    content — the group rollup, §5.3, then reads the group as attested-current only when both are.)

### 6.3 CLI — `--attest-as` on every verb
- New **`medication-attest <medication_id>`** — post-hoc per-thread sign-off. Flags: `--attester-key` /
  `--attester-passphrase` (reuse `load_attester_key` — a human key distinct from the node's operational key),
  `--basis`, `--note`. `attester_is_enrolled_human` pre-check (legibility; the db/005 gate is the real gate).
  Clear error if the thread is not local.
- **Every existing verb** (`medication-assert` / `-cease` / `-change-dose` / `-correct-dose` /
  `-reconcile` / `-separate`) gains **`--attest-as <attester-key>`** (+ `--attest-passphrase`, `--basis`,
  `--note`) — the author-time convenience: atomic verb-then-vouch via the §6.2 author-time path. Absent ⇒
  unchanged device-additive behaviour. One shared clap flag group + one shared "resolve attester" helper
  (loads the key, pre-checks human-ness) keeps the six call sites DRY; cross-flag validation mirrors
  `identify-patient` (`--attest-as` ⟺ its passphrase; a passphrase with nothing to attest is refused loudly).
  `-reconcile`/`-separate` vouch for both subject threads.

### 6.4 Companion refactor — split `crates/cairn-node/src/medication.rs`
Already 621 lines; this slice adds ~150. Split into a module dir mirroring `cairn-event::medication`:
`mod.rs` (+ shared `EventBody`-assembly helper), `assert.rs`, `cessation.rs`, `dose.rs`, `reconciliation.rs`,
`attestation.rs`. Pure code-move (tests green at each step) + the new module. This is the file the slice
edits, so it is in-scope cleanup (house rule 4: files under 500 lines), not drive-by refactoring. `main.rs`
(1744 lines) is noted but out of scope.

---

## 7. TDD plan (subagent-driven SDD, ~6–8 tasks)

Pure first, then DB-gated (`crates/cairn-node/tests/medication_attestation.rs`):

1. **Floor** — accepts well-formed; rejects blank `medication_id` / malformed `reviewed_commitment` / empty
   twin / negative `reviewed_count` (floor rejections).
2. **Responsibility gate** — un-attested refused; **agent-attested refused**; valid human token accepted
   (the `v_bears` path; mirrors repudiate's human-floor tests).
3. **Post-hoc happy path** — assert → attest → `medication_thread_attestation` shows the attester +
   `stale=false`.
4. **Staleness core + the gap** — attest → later `dose-change` → `stale=true`; **and** attest → a *lower-HLC*
   content event arrives out of order → **still `stale=true`** (the test that proves the commitment fix over
   a position pin — the design's load-bearing property).
5. **Group rollup** — two threads reconciled; group `attested & current` only when both members are; one
   member stale/unattested → group not current; a singleton reduces to its thread.
6. **Author-time atomic, every verb** — the author-time path on `assert` and on one single-thread mutating
   verb (`dose-change`) → the thread is attested-current in one txn; a forced attestation rejection rolls the
   verb back (nothing written). Plus the **pair** case: `reconcile --attest-as` → **both** subject threads
   attested-current in one txn.
7. **Supersede-not-retract** — attest a thread → author a `dose-correction` ("acted in error, correct is X")
   → the prior attestation reads `stale=true` (commitment changed), the erroneous vouch is still in the
   record (retained-set/`medication_attestation` row intact), and a fresh attestation of the corrected state
   reads `stale=false`. Proves the intended workflow: responsibility is never withdrawn, only superseded.
8. **Offline-first / orphan** — the floor accepts an attestation for a not-local thread; it renders nothing
   until the thread's events arrive, then computes staleness correctly.
9. Live e2e CLI smoke: assert → attest (vouched, current) → change dose (stale / re-review) → re-attest
   (current); a `-reconcile --attest-as` vouches for both threads.

Full workspace green (fmt + clippy `--workspace -D warnings` + `cargo test --workspace`) + mkdocs.

---

## 8. Honest limits & deferred

- **Reviewed = the set present on the attester's node at review time.** The commitment cannot cover an event
  that exists on no reachable node yet (reviewing the future). The §5.13 background re-review sweep is the
  backstop. Documented in §5.1, not silently ignored.
- **Responsibility is never retracted, only superseded** (the maintainer's clinical call — principle 1/2).
  There is **no de-attestation / withdraw-vouch event**, and there deliberately never will be: a clinician who
  vouched in error does not un-vouch — they author a *corrective* clinical event ("I acted in error doing X;
  the correct value is Z" — a `dose-correction` / `cessation` / new assertion), which changes the thread's
  content set → flips the prior attestation **stale** → prompts re-review → they re-vouch for the corrected
  state. The erroneous vouch stays in the record; the accountability trail is *"Dr X vouched for the wrong
  value at T1, corrected and re-vouched at T2,"* never an erasure. This is precisely what the commitment-based
  staleness makes work, and it is *why* no withdraw event is needed.
- **Group rollup is conservative** (all-active-members-current). A "partially attested group" nuance
  (which member is stale) is a future read-surface refinement.
- **Whole-list sign-off is composed, not a distinct event** (N thread attestations). A single "list reviewed
  at T" summary event is a future convenience if the worklist wants one.
- **`reviewed_count` is a legibility hint only** — the commitment is the sole staleness authority (a
  disagreeing count cannot cause unsoundness).
- **Perf:** the commitment view recomputes a SHA-256 per thread per read. Negligible for a patient's list
  (tens of threads, a handful of events each); if a hot path emerges, memoize into the overlay table. Noted,
  not premature-optimized.

## 9. ADR-0049 — commitment-based sign-off currency (confirmed)

**Decision (maintainer-confirmed): write ADR-0049.** The **commitment-based staleness / currency-of-a-sign-off**
pattern is a reusable precedent: a point-in-time human review binds to the exact *set* it reviewed, and "still
current?" is a convergent set-commitment compare; responsibility is superseded (by correcting the record),
never retracted. It directly advances [issue #163](https://github.com/cairn-ehr/cairn-ehr/issues/163)
(asserted-since vs confirmed-current-as-of) and binds every future clinical stream's "re-affirmation /
sign-off currency" work. Spec bump v0.49 → v0.50 (`index.md` only). ADR home: §3.15/§3.16 (refines ADR-0007
principle 10; reuses the db/005 gate).

**Post-implementation correction:** the `medication_thread_head` view (§5, item 1 above) — this design's
proposed ADR-0045-style collation-pinned position read — was **dropped** during implementation; the
commitment compare is the sole staleness authority and needs no separate head view. As shipped, `db/034`
contains no `COLLATE` clause at all: its one ordering tiebreak sorts `content_address`, which is BYTEA
(inherently byte-ordered), so ADR-0045 (a TEXT-key discipline) is not actually reused. See ADR-0049 as
merged for the corrected Refines/Consequences wording.
