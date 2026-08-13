# Design — safety-projection emission (§5.9 slice B, issue [#375](https://github.com/cairn-ehr/cairn-ehr/issues/375))

- **Date:** 2026-08-13
- **Issue:** [#375](https://github.com/cairn-ehr/cairn-ehr/issues/375) (§5.9 part B), carrying
  [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) (the class must be CARRIED, never re-derived)
- **Canonical spec home:** [identity §5.9](../../spec/identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
- **Builds on:** [ADR-0062](../../spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md)
  (part A — the grade this slice consumes) · [ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md)
  decision 4 (carry the class) · [ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md)
  (born-sealed bodies; the seal boundary this design reuses) ·
  [ADR-0006](../../spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)
  (the safety projection's original shape) · [ADR-0005](../../spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md)
  (the severity ladder the coarsening ladder mirrors; *deletion is best-effort and declared*)
- **Will produce:** ADR-0063

---

## 1. Scope, and what this slice deliberately is not

[#232](https://github.com/cairn-ehr/cairn-ehr/issues/232) is four subsystems. Part **A** (the sensitivity
stream) shipped as Slice 65. This slice builds **B** and files the rest:

| | Piece | State |
|---|---|---|
| **A** | the sensitivity stream: graded assertions + the effective-grade projection | shipped (ADR-0062) |
| **B** | **safety-projection emission — de-identified class + severity, coarsened by the grade** | **this slice** |
| **C** | sequester: custody narrowing (re-wrap / withdraw DEKs) | [#376](https://github.com/cairn-ehr/cairn-ehr/issues/376), unblocked by Slice 66 |
| **D** | break-glass: audited key-*use*, partition-honest | [#377](https://github.com/cairn-ehr/cairn-ehr/issues/377), blocked on C |

**Part B still enforces nothing, and saying so is again part of the decision.** It emits and coarsens a
*signal*; it withholds no content. What it changes versus part A is that the grade now **does something**:
before this slice a graded event's safety relevance was invisible, so confidentiality and safety were
simply not composed.

Deliberately **not** in this slice:

- **No enforcement / custody narrowing.** That is part C.
- **No UI warning surface.** The read model returns the ingredients; the decision-support surface that
  fires *"⚠ Grade X interaction with confidential content"* lands with the UI slice that consumes it.
  This slice's read surface is a CLI verb, enough to test end-to-end.
- **No deployment-configurable grade→rung map.** §5.9 calls the ladder *policy-configured*; this slice
  ships a hardcoded monotone default keyed on `cairn_sensitivity_rank` and files the override table
  (§12). Shipping the override now would be surface without a caller.
- **No rung-4 oblivion.** §5.9 says the safety projection is shreddable *only* at rung 4; rung 4 is not
  built, so the signal is simply permanent today. Stated, not silently assumed.
- **No drug knowledge.** `safety_class_map` ships **empty** (§7).

---

## 2. Decisions this slice takes (→ ADR-0063)

1. **The seal boundary is the coarsening boundary.** The precise class is captured pre-seal *inside* the
   sealed payload; a rung chosen by the then-effective grade is emitted in the clear on the signed
   envelope.
2. **Coarsening binds at emission, and is re-applied at read.** Emission is the control (it is the only
   thing that binds a raw-SQL client on a peer); read is the local defence against a peer that emitted a
   finer rung than this node's grade permits.
3. **An unrecognised severity ranks MAX**, inheriting ADR-0062 decision 2's inversion — for a *safety*
   signal, MAX means "treat as most severe", which is again the safe direction.
4. **The signal rides the append-only `event_log` row, not a projection table.** Coarsen-but-survive
   after a crypto-shred then falls out of the storage model instead of needing a scrub exemption.
5. **`safety_class_map` ships empty**, exactly as `sensitivity_category_map` does: Cairn ships the lookup
   mechanism and never the drug knowledge.
6. **A grade raised after authoring cannot claw back an already-replicated rung** — declared, not hidden.
7. **The signal's floor is local-door-only, and the read model is total.** A defect in a de-identified
   advisory field must never cost the clinical event it rides on (§7a).

---

## 3. Wire — one additive plaintext field, one sealed sibling

### 3a. The clear field (`EventBody.safety`)

Appended **trailing** on `EventBody`, after `clock_grade` — the ADR-0058 precedent, so existing signed
bytes are never re-encoded and every existing content address is unchanged (principle 11 / ADR-0012):

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub safety: Option<SafetySignal>,
```

`skip_serializing_if` means a `None` emits **no CBOR key at all**, so adding the field is byte-identical
for every event that does not carry one.

Three rungs, mirroring §5.9's ladder (*precise class → "confidential medication, severity X" →
"confidential content, break glass"*):

```json
{"rung":"precise",   "class":"rh-sensitizing", "severity":"high"}
{"rung":"kind",      "severity":"high"}
{"rung":"existence"}
```

**Why `kind` carries no `kind` field.** §5.9's middle rung reads *"confidential **medication**, severity
X"*. The word *medication* is already in the clear: `event_log.event_type` is a plaintext column and
`clinical.medication.asserted` says it outright. A `kind` field would restate what the row already
publishes, so the rung carries only what is genuinely additional — the severity. The read model composes
the human sentence from `event_type` + `rung` + `severity`.

**Why `existence` still emits a row.** *Coarseness varies; existence never disappears* (§5.9's
safety-floor invariant). `{"rung":"existence"}` is the assertion *"there is a safety-relevant signal on
this event, and you are not cleared to see what"* — which is precisely what makes break-glass a rational
act rather than a fishing expedition.

### 3b. The sealed sibling (`payload.safety`)

The precise claim is captured pre-seal and travels **under the same DEK as the body it describes**:

```json
"safety": {"class": "rh-sensitizing", "severity": "high"}
```

This is #294 / ADR-0059 decision 4 made real: the class is computed on the coding node, which by
construction had a coding authority in hand, and is never re-derived by a reader.

### 3c. Why two tiers rather than one

Emitting the precise class in the clear and coarsening only at read was considered and **rejected as a
leak**, not a simplification. For the exact cases §5.9 exists for, the class *is* the disclosure:
*"Rh-sensitizing event"* in the clear is *"this patient had a termination or a miscarriage"*; an
antiretroviral interaction class is *"this patient has HIV"*. A raw-SQL client on any peer reads it —
the projection-layer theatre ADR-0062 refused to ship, one subsystem over.

Sealing the class *only*, with nothing in the clear, was also rejected: the custody-less node is exactly
the node §5.9 is written for, and it would get no signal at all.

The two-tier shape makes every degradation fall out of the seal boundary rather than needing machinery:

| reader | gets | why |
|---|---|---|
| custody, no drugref | the precise class from the sealed payload | #294's promise: carried, never re-derived |
| no custody (sequestered — part C) | the clear rung | the coarsening actually binds |
| after a rung-3 crypto-shred | the clear rung, permanently | `cairn_execute_shred` never touches `event_log` |
| grade withdrawn later | custody-holders recover precision by reprojecting the sealed body | the custody-less peer stays coarse, and was never entitled |

---

## 4. Where the class comes from

There is no drugref today, and this slice does not add one. The class enters through **one seam with two
callers**, the same shape ADR-0062 decision 8 gave the category blacklist:

```
cairn_safety_class_candidate(p_coding jsonb) RETURNS TABLE (class text, severity text)
```

- `p_coding` is the `{system, code, display}` object of ADR-0059 decision 2.
- It reads `safety_class_map`, which **ships empty** (§7).
- It is **pure and authorless**: it returns a candidate to its caller and writes nothing. The caller is
  the authoring verb, pre-seal.
- It is **never called by a reader.** That is the whole of #294, expressed as a call-site rule the test
  suite pins.

**Uncoded medications get no signal at all** — not `{"rung":"existence"}`. ADR-0059 decision 4 is
explicit: *"For an **uncoded** medication there is no class on **any** node, drugref present or not — that
is the principle-4 'little white pill' floor being honest, not a degradation."* Emitting an existence
marker for every uncoded medication would manufacture a signal from nothing and reproduce §5.12's
alert-fatigue disease on day one.

**A coding with no class in the map likewise emits nothing.** The map returning no row means this
deployment's coding authority has no opinion about this substance, which is an absence of knowledge, not
a graded secret. (Contrast: a coding *with* a class on a `sequestered` chart emits `{"rung":"existence"}`
— there the signal exists and is being blurred.)

---

## 5. The two coarsenings, and why both are load-bearing

### 5a. At emission — the control

In `seal_sign_submit`, before the seal:

```
precise  := body.payload.safety                        (absent ⇒ nothing to do, return)
rank     := cairn_prospective_sensitivity(patient, thread)   → cairn_sensitivity_rank
rung     := cairn_safety_rung_for_rank(rank)                 (pure, monotone)
body.safety := coarsen(precise, rung)                        (pure, in cairn-event)
```

The default map, keyed on rank so ADR-0062's open vocabulary and unknown-ranks-MAX are inherited for
free rather than re-spelled:

| grade | rank | rung |
|---|---|---|
| `routine` (or no assertion) | 0 | `precise` |
| `sensitive` | 10 | `kind` |
| `restricted` | 20 | `existence` |
| `sequestered` | 30 | `existence` |
| anything unrecognised | `MAX` | `existence` |

Monotone non-decreasing in rank by construction: a higher grade can never disclose more. A future grade
value interposed at rank 15 lands on `kind`; one at 25 lands on `existence`; an unrecognised one lands on
`existence` without anyone remembering to add it — the same safe-default-by-omission discipline
ADR-0062 decisions 2 and 10 use.

**This is the only coarsening that binds a raw-SQL client**, because it decides what is put on the wire
at all.

**The read-then-sign race, named because it is structural.** The prospective grade is read in one query
and the event is signed and submitted in another, so a grade raised in between yields a rung one step too
fine. The window cannot be closed by moving the decision into `submit_event`: the rung must be inside the
**signed** bytes, and signing happens in the daemon where the DEK lives. The window is milliseconds, the
direction of the error is bounded by §5b's read-time coarsening on every node that later holds the
grade, and re-authoring is not the remedy — re-asserting the *grade* is. Declared rather than defended
against.

### 5b. At read — the local defence

```
cairn_event_safety(event_id) →
    rung := coarser_of( emitted rung,
                        cairn_safety_rung_for_rank(rank of cairn_effective_sensitivity(event_id)) )
```

Needed because **an event arrives carrying whatever rung its authoring node chose**. An older peer that
predates this slice's grade, a peer whose custody differs (the grade is node-relative — ADR-0062
decision 9), or a hostile peer, can all deliver `{"rung":"precise", ...}` on an event whose chart is
`sequestered` here. Refusing it at the apply door would fork the event set (the
[#342](https://github.com/cairn-ehr/cairn-ehr/issues/342) trap, and ADR-0062 decision 7's argument), so
we **admit and coarsen locally**.

Neither coarsening alone is sufficient, and the reasons are different: emission cannot control a peer's
bytes, and read cannot un-publish a byte already on the wire. The ADR states both, because a future
reader "simplifying" away either one reopens a distinct hole.

### 5c. The limitation this design accepts, declared

**A grade raised after authoring cannot claw back an already-replicated rung.** A `precise` rung emitted
while a chart was `routine` remains readable on every node that already holds the event, even after the
chart is graded `sequestered`. Read-time coarsening blurs it for every honest consumer, and that is all
any mechanism can do.

This is ADR-0005's *deletion is best-effort and declared, never guaranteed* — the ninth principle's
corollary — and it is the same hazard ADR-0062 already declared for a withdrawal's clear-text rationale.
It is **why emission-time coarsening is the control rather than an optimisation**: the moment of
authoring is the only moment at which a decision about what to publish can actually bind.

Partial mitigation, worth naming because it is free: a node with custody can recover precision from the
sealed payload, so the *loss* from over-coarsening at emission is bounded to nodes that hold neither
custody nor a coding authority — exactly the nodes not entitled to the class.

---

## 6. Data model

### 6a. The column, not a projection table

```sql
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS safety JSONB;
```

Additive, `ADD COLUMN IF NOT EXISTS` (does not fire the append-only trigger), the same precedent as
`clock_grade`, `attestation` and `attester_key`.

**Why not a projection table.** §5.9 requires the safety projection to *outlive the body it protects* —
to coarsen but survive a rung-3 crypto-shred. A projection table would have to be **explicitly exempted**
from `cairn_execute_shred`'s scrub, which is a standing invitation for a future reviewer to "fix" the
inconsistency and silently delete the one signal the spec says must survive. On the append-only row it
survives because `event_log` is never touched by a shred — the guarantee is structural rather than
remembered. It also needs no apply function, no ADR-0057 registry entry, and no reprojection, so it
touches none of the four registry row-count pins.

⚠️ **Adding a column to `event_log` invalidates positional `ROW` literals in stale developer databases.**
`born_sealed_schema.rs` builds `event_log` ROW values positionally; a dev cluster carrying the old column
list fails with *"invalid input syntax for type bigint"*. The local `cairn_test` / `cairn_test2` /
`cairn_test3` databases must be dropped and recreated after this migration. CI is immune (fresh DBs).

### 6b. `safety_class_map` — the empty lookup

```sql
CREATE TABLE IF NOT EXISTS safety_class_map (
    system   TEXT NOT NULL,
    code     TEXT NOT NULL,
    class    TEXT NOT NULL,
    severity TEXT NOT NULL,
    note     TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (system, code)
);
```

Keyed on the **pair** `(system, code)`, never a bare `code` — ADR-0059 decision 5's argument applies
unchanged: once `drugref-clinical-drug` exists beside `drugref-moiety`, a bare-`code` key would collide
across composition-tree levels.

Ships **empty**, with the SQL mirror asserting emptiness, for exactly ADR-0062 decision 5's reason: a
seeded row is an un-reviewable policy choice smuggled in as infrastructure. This table is also the seam
the future drugref slice populates.

### 6c. Functions

| Function | Purpose |
|---|---|
| `cairn_safety_severity_rank(text) → int` | `none` 0 · `low` 10 · `moderate` 20 · `high` 30 · `critical` 40 · **ELSE MAX** |
| `cairn_safety_rung_rank(text) → int` | `precise` 0 · `kind` 10 · `existence` 20 · **ELSE MAX** (coarsest) |
| `cairn_safety_rung_for_rank(int) → text` | the §5a monotone map from a sensitivity rank |
| `cairn_check_safety_signal(jsonb) → void` | the structural floor (§8) |
| `cairn_prospective_sensitivity(uuid, uuid) → (grade, subject_kind, content_address)` | the grade for an event **not yet written** (§6d) |
| `cairn_event_safety(uuid) → (rung, class, severity, event_type, grade, subject_kind)` | the read-time-coarsened signal |
| `cairn_patient_safety(uuid) → setof the above` | every standing signal on a chart |
| `cairn_safety_class_candidate(jsonb) → (class, severity)` | the empty-map lookup, callers pre-seal only |

Both rank functions carry the same shouting comment ADR-0062's does, for the same reason: the `ELSE`
is the decision, and "tidying" it into consistency with `cairn_clock_grade_rank`'s `ELSE 0` reopens a
leak in one case and mutes a critical-severity warning in the other.

### 6d. `cairn_prospective_sensitivity` — the write-time grade

`cairn_effective_sensitivity` takes an `event_id`, and at emission time the event does not exist yet. The
prospective form takes `(patient_id, thread_id)` and mirrors section 11's arms **minus the event arm** —
an event about to be written can carry no event-scoped assertion.

The duplication is real and is the one drift risk in this slice. It is mitigated three ways: the two
functions live adjacent with a cross-reference comment in each; both delegate to the single
`cairn_sensitivity_standing` definition of *what still applies*; and a test pins them equal —
`prospective(patient, thread) == effective(event)` for an event on that thread with no event-scoped
assertion standing.

---

## 7. The floor — local door refuses, remote door admits, read model is total

### 7a. Why this is NOT "structural, therefore both doors"

ADR-0062 E2 draws the line at *structural* (the shape of the claim — safe at both doors) versus
*ceremony* (who authored it — local only). Read naively, the safety field's shape check is structural and
belongs at both doors. **That reading is wrong here, and the reason is blast radius, not category.**

A sensitivity assertion *is* an event: refusing a malformed one drops one assertion, and the chart is
otherwise untouched. The safety signal is a **field on a clinical event**: refusing it at the apply door
drops the *medication assertion it rides on* off this node's chart entirely. A defect in a de-identified
advisory signal would then destroy clinical content — [ADR-0060](../../spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)'s
*a defect on one line never invalidates another*, and its harder corollary: **the system may fail to
record an order, but it may never cancel one.**

So the split follows the `clock_grade` precedent (`db/040`) exactly — an envelope-level graded field is
constrained where it is *minted* and read permissively where it *arrives*:

| | Local door (`submit_event`) | Remote door (`apply_remote_event`) |
|---|---|---|
| malformed / self-contradictory `safety` | **refuse** | **admit**, store verbatim |
| the signal a reader then sees | — | coarsest (§7c) |

Refusing at the local door is the ADR-0062 E5 argument one field over: a peer that sent a
self-contradictory signal has *already* published those bytes, so refusing at apply un-discloses nothing
and would only fork the event set (the [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342) trap) —
here at the price of clinical content as well. Stopping nodes from **authoring** the contradiction is the
only thing a door can accomplish.

### 7b. `cairn_check_safety_signal` — the local-door check

- `safety` absent ⇒ pass (the overwhelmingly common case).
- Present ⇒ must be a JSON object with a non-empty `rung` string.
- `rung = 'precise'` ⇒ `class` must be present and non-empty.
- `class` present at any rung other than `precise` ⇒ **refuse.** A body claiming
  `{"rung":"existence","class":"rh-sensitizing"}` publishes the class while asserting it is concealed.
- `severity`, when present, must be a non-empty string. No `CHECK` domain — open vocabulary, principle 11.

Called from the envelope well-formedness step of `db/005_submit.sql` only. It is an envelope-level field
like `clock_grade` and `attachments`, not a per-type body field, so it does **not** go through the
ADR-0048 twin-check registry (and therefore moves no registry row count).

### 7c. The read model is total, and that is what makes the leniency safe

`cairn_event_safety` never trusts the stored shape:

- an unrecognised or missing `rung` reads as `existence` (`cairn_safety_rung_rank`'s `ELSE` MAX);
- **`class` is surfaced only at `rung = 'precise'`** — a class sitting beside a coarser rung is ignored,
  always, whatever the row holds;
- an unrecognised `severity` ranks MAX.

`event_log.safety` therefore stays an honest **derived view of the signed bytes** — never sanitized on
the way in, which would make the column disagree with `signed_bytes` — while the sanctioned read surface
cannot be made to surface a class the rung does not license. Admitting the contradiction and refusing to
*act* on it is ADR-0056's *custody is total; interpretation is deferred; power is earned*, applied to a
field.

---

## 8. Emission seam

`seal_sign_submit` (`crates/cairn-node/src/medication/sealed_submit.rs`) is the one path every clinical
verb submits through, and already carries the seal-then-sign discipline for exactly this reason. The
coarsening goes **there**, not in each verb, so no future clinical verb can forget it.

```
apply_author  →  [coarsen safety]  →  seal_and_sign  →  ensure_unwrap_key  →  submit_event
```

The impure half (reading the prospective grade) is one query in `cairn-node`; the pure half
(`coarsen(precise, rung) -> SafetySignal`) lives in `cairn-event::safety` and is exhaustively unit-tested
without a database.

The two-thread reconcile/separate verbs use `seal_and_sign` directly and carry no coding, hence no
safety claim — unchanged, and pinned by a test so the omission is deliberate rather than incidental.

**Which verbs populate `payload.safety`:** the three that carry a coding —
`clinical.medication.asserted` (inline coding, slice 6a), `clinical.medication-coding.asserted` and
`clinical.medication-coding-correction.asserted` (overlays, slice 6b). A `CodingClaim::Strike` correction
carries no coding and therefore no class, which is correct: a struck coding is back to *not-yet-coded*.

---

## 9. Read surface

A `patient-safety <patient-uuid>` CLI verb, printing one line per standing signal:

```
⚠ moderate — statin interaction class            clinical.medication.asserted   (routine)
⚠ high     — confidential content, break glass   clinical.medication.asserted   (sequestered, chart-wide)
```

It **names nothing** beyond the class the rung permits: no agent, no diagnosis, no scope key. The
parenthesised grade and winning subject come straight from `cairn_effective_sensitivity`, honouring
ADR-0062 decision 8's control 3 (*the read surface always names which subject won*) — including its
erratum-E6 correction that the catch-all arm reports `coarsened`, and that the *"did anything win"* test
is `content_address IS NOT NULL`, never `subject_kind <> 'none'`.

---

## 10. Paper-parity benchmark (§1.2, CLAUDE.md house rule 7)

**Paper counterpart.** The paper chart with a confidential episode in a sealed envelope stapled inside
the folder, and an allergy/interaction sticker on the front sheet. The next clinician reads the front
sheet — **N = 1 human act** — and learns *"there is something here that interacts; the detail is in the
envelope"*, without opening it.

**Steps.** Paper `N = 1` → architecture-forced `M = 0` additional human acts: the class is captured at
coding time from the type-ahead the clinician is already using (ADR-0059's `M = N` coding benchmark), the
rung is chosen automatically at write, and the signal is read at chart open. UI bundling target `K = 0` —
the signal must appear *with* the chart, never behind a click. `M < N`, so no architecture defect to
file.

**Time + cognitive load.** Budget: the signal must be on screen within the med-list's existing chart-open
budget (no additional round trip — `cairn_patient_safety` is one query on the same connection), and must
add **zero** clicks. Cognitive load must *fall* versus paper: the front-sheet sticker requires the
clinician to notice and interpret it, whereas the warning is composed into a sentence naming the
severity. **Measurement is owed by the UI slice that first renders it** — this slice exposes only a CLI
verb. If either the query cost or the click count exceeds budget there, that is the finding: file it, do
not adjust the budget.

---

## 11. Test plan (TDD — every test red first)

**Pure, no DB** (`cairn-event`):
1. `coarsen` at each rung drops exactly the fields that rung forbids, and is total over the ladder.
2. A `None` safety serialises to byte-identical CBOR versus a pre-field body — the additive-evolution pin.
3. A body carrying `safety` round-trips through sign/verify unchanged.
4. An unrecognised severity ranks MAX (Rust side of the ladder).

**In-DB floor** (`db/049` + the SQL mirror):
5. `cairn_check_safety_signal` admits absence, admits each well-formed rung, refuses a missing `rung`,
   refuses `precise` without a `class`, refuses a `class` at `kind`/`existence`.
6. **The door asymmetry is pinned, not commented** (§7a): the same self-contradictory body is refused at
   `submit_event` and **admitted** at `apply_remote_event` — and the clinical content it rides on lands
   on the chart, which is the half that actually matters.
7. `cairn_safety_severity_rank` / `cairn_safety_rung_rank` put an unrecognised value at MAX.
8. `cairn_safety_rung_for_rank` is monotone non-decreasing across the whole ladder including MAX.
9. `safety_class_map` ships empty.

**Emission** (`cairn-node`):
10. A coded assert on a `routine` chart emits `precise`; the sealed payload carries the class.
11. The same assert on a chart graded `sensitive` emits `kind` — class **absent from the clear field**,
    still present under the seal.
12. `restricted` and `sequestered` both emit `existence`; an unrecognised grade also emits `existence`.
13. An **uncoded** medication emits no safety field at all.
14. A coded medication whose `(system, code)` is not in the map emits no safety field.
15. `prospective(patient, thread) == effective(event)` for an event on that thread with no event-scoped
    assertion — the anti-drift pin for §6d.

**Read** (`cairn-node`):
16. **#294's obligation:** a node holding the event and its custody, with `safety_class_map` **empty**
    (i.e. no local coding authority at all), still reports the precise class — proving it was carried,
    not re-derived. This is the test the medication coding slice owed and could not write.
17. A peer event arriving with `{"rung":"precise"}` on a locally-`sequestered` chart reads back as
    `existence` — read-time coarsening, admitted at the door not refused.
17b. A peer event arriving with a **self-contradictory** `{"rung":"existence","class":"…"}` applies, and
    `cairn_event_safety` surfaces **no class** — the §7c totality rule, which is what makes §7a's
    leniency safe rather than merely lenient.
18. **Coarsen-but-survive:** after `cairn_execute_shred` of the event, `cairn_event_safety` still returns
    a signal, and the class is gone with the sealed body.
19. A chart-wide `sequestered` grade coarsens every signal on the chart (ADR-0062 decision 8's blast
    radius, made visible now that part B exists).
20. The read surface names the winning subject, and uses `content_address IS NOT NULL` as its
    did-anything-win test (the E6 trap).

**Cross-cutting:**
21. `cairn-sync`'s SCHEMA subset loads `db/049` and **drives** it — the mistake
    [#386](https://github.com/cairn-ehr/cairn-ehr/issues/386) records against db/048's subset test, not
    repeated here.
22. The reconcile/separate path emits no safety field (deliberate omission, pinned).

---

## 12. Files, and the repo's standing traps

**New:** `db/049_safety_projection.sql` · `db/tests/049_safety_projection_test.sql` ·
`crates/cairn-event/src/safety.rs` · `crates/cairn-node/src/safety.rs` ·
`crates/cairn-node/tests/safety_*.rs` · `docs/spec/decisions/0063-*.md`

**Modified:** `crates/cairn-event/src/lib.rs` (the trailing field) ·
`crates/cairn-event/src/schema_generation.rs` (48 → 49) · `db/005_submit.sql` (the floor call **and** the
`safety` column in the insert) · `db/020_apply_remote_event.sql` (the column **only** — no floor call,
§7a) · `crates/cairn-node/src/db.rs` **and**
`crates/cairn-sync/src/main.rs` (both SCHEMA lists) · the three medication verb builders ·
`sealed_submit.rs` · `crates/cairn-node/src/main.rs` (the CLI verb) · `docs/spec/identity.md` §5.9 ·
`docs/spec/index.md` (spec version) · `docs/spec/decisions/README.md`

**Traps this slice walks into, from the repo's own scar tissue:**

- **Stale dev DBs.** The `event_log` column add breaks positional `ROW` literals in
  `born_sealed_schema.rs`. Drop and recreate `cairn_test`/`2`/`3` on :5532 before running the gate.
- **Two schema lists.** `db/049` must go in `cairn-node/src/db.rs` *and* `cairn-sync/src/main.rs`. A door
  carrying a rule it cannot satisfy is Slice 64's and Slice 66's shared lesson, both directions.
- **`tests/common` helpers are pinned twice.** A new `pub fn` in `cairn-node/tests/common/mod.rs` must
  also be added to `identity_scaffolding_shared.rs`'s hand-written expected-helper array.
- **Cross-crate call sites.** Changing `seal_sign_submit`'s signature breaks
  `cairn-sync/tests/clinical_pull.rs`. Only a full-workspace `cargo test` catches it.
- **No registry counts move.** This slice registers no event type and no projection, so the four
  row-count pins (`twin_registry.rs`, `db/tests/034`, `projection_registry.rs`, `db/tests/039`) are
  untouched — asserted here so a reviewer can check the claim rather than assume it.
- **UUIDs bind as text** (`$1::text::uuid`), and DB-gated tests take `db::test_serial_guard` *before*
  `connect_and_load_schema`.

---

## 13. Follow-ons to file

1. **The deployment-configurable grade→rung map** — §5.9's *policy-configured* ladder, deferred here as
   surface without a caller. Must stay monotone non-decreasing in rank; that constraint is mechanism, not
   policy.
2. **Rung-4 oblivion for the safety signal** — §5.9 says the projection is shreddable only at rung 4.
   Today it is permanent, because rung 4 does not exist.
3. **The UI warning surface + its §1.2 measurement** (§10's owed budget).
4. **drugref populates `safety_class_map`** — the natural consumer of §6b's seam, and part of the
   term→anchor lookup slice.
5. **Non-medication clinical streams** — this slice's emission seam is generic, but only medication verbs
   carry a coding today. Notes/orders/results inherit the seam when they land.
