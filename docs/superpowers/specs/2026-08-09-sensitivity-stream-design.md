# Design — the sensitivity stream (§5.9 slice A, issue #232 part A)

- **Date:** 2026-08-09
- **Owes:** [#232](https://github.com/cairn-ehr/cairn-ehr/issues/232) (part A only)
- **Implements:** [ADR-0006](../../spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)
  decision 3 · [spec §5.9](../../spec/identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
- **Lands:** ADR-0062 (six decisions ADR-0006 leaves open — §2 below); spec **v0.63 → v0.64**;
  `SCHEMA_GENERATION` **47 → 48** (`db/048`)

---

## 1. Scope, and what this slice deliberately is not

#232 is four subsystems, not one. This slice builds **A** and files the rest:

| | Piece | Status after this slice |
|---|---|---|
| **A** | Sensitivity stream: graded append-only assertions + effective-grade projection | **built here** |
| **B** | Safety-projection emission (de-identified class + severity, coarsened by the grade) | issue filed; blocked on A and [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) |
| **C** | Sequester: custody narrowing (re-wrap / withdraw DEKs) | issue filed; blocked on A **and [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231)** |
| **D** | Break-glass: audited key-*use*, partition-honest | issue filed; blocked on C |

**C must not be built before #231.** `cairn-sync serve` verifies an unwrap-key certificate against its
own signature and self-consistency only — it does not check the cert's `kid` against the admitted-peer
trust set, so transport is the sole gate on read-custody. Narrowing a body's custody to two named
clinicians while that hole stands is defeated by asking the serve port for the DEK: protection real in
the projection layer, absent at the wire. Recorded here because the ordering is not obvious from #232.

**This slice enforces nothing.** It computes and reports a grade. Anything that *withholds* content on
the strength of a grade, with no custody narrowing beneath it, is the same theatre one layer up — a
projection-layer filter is not a floor (principle 12), and a raw-SQL reader walks straight past it.

**Non-goals:** no safety projection, no custody change, no break-glass, no episode entity, no UI.

---

## 2. Decisions this slice takes (→ ADR-0062)

ADR-0006 decision 3 fixes the *shape* — graded, multi-source, append-only, effective grade is a
projection, highest standing assertion wins, declassification is an authorized overlay never an
erasure. It leaves six things open, each decided here:

1. **Subject granularity** — an assertion names an `event`, a `thread`, or a `patient`; the effective
   grade of an event is the **max** over standing assertions on all three (§4).
2. **An unrecognized grade ranks MAX**, inverting the `clock_grade` precedent (§5).
3. **Declassification is withdraw-by-reference**, with the ceremony enforced at the **local authoring
   door only** (§6). A corollary the ADR must also carry: **the effective grade is node-relative**,
   because thread membership is knowable only with custody (§10b) — the ADR-0052 §9 pattern again.
4. **Sensitivity assertions are plaintext by necessity** — extending ADR-0052 §2's list (§3).
5. **The matched category never travels on the wire** (§3).
6. **ADR-0043's "agent advisories are dismissable by anyone" does not reach a protective auto-tag** —
   dismissing one is a lowering and routes through the ceremony (§6).

---

## 3. Wire — two event types, both plaintext

```
sensitivity.grade.asserted
sensitivity.grade-withdrawal.asserted
```

### Plaintext by necessity (extends ADR-0052 §2)

ADR-0052 §2 lists what stays plaintext because the machinery binds on it. Sensitivity assertions join
that list, for a reason with the same shape as the shred tombstone's: **a node must read the grade in
order to coarsen, and coarsening is exactly what a node holding no custody of the graded body must
still do.** Sealing the grade under the key it governs is circular — the node that most needs to know
"treat this carefully" is the one that cannot open it.

### The category must never travel

ADR-0006 decision 4 warns that plaintext scope keys can be the whole disclosure. A body carrying
`category: "termination-of-pregnancy"` in a plaintext, unconditionally-replicated event **is** the
disclosure the mechanism exists to prevent. So the assertion carries subject, grade and provenance —
never the matched category. Where the tag came from is node-local audit at most.

### Bodies

`sensitivity.grade.asserted` payload:

| Field | Required | Notes |
|---|---|---|
| `subject_kind` | yes | `event` \| `thread` \| `patient`. **Not a closed CHECK** — see §4. |
| `subject_id` | yes | UUID string |
| `grade` | yes | open vocabulary, non-empty |
| `source` | yes | `human` \| `advisory` — provenance of the tag, not an authority claim |
| `rationale` | **only when `subject_kind = 'patient'`** | free text; see the friction rule in §6 |

`sensitivity.grade-withdrawal.asserted` payload:

| Field | Required | Notes |
|---|---|---|
| `withdraws` | yes | hex `content_address` of the assertion being withdrawn |
| `rationale` | yes | free text — the audited *why* |

The **owner** of a withdrawal is not a payload field: it is the ADR-0053 bound human author. Reusing
the existing authorship binding beats inventing a `owner` string that nothing verifies.

> **`rationale` is clear text, forever, and it replicates.** It sits in a plaintext event by
> necessity (the grade must be readable without custody), so a rationale naming the condition —
> *"patient consented after her termination follow-up"* — leaks precisely what the grade protects. The
> UI must say so at the point of entry. A sealed-rationale variant is a follow-on issue (§12), not
> this slice.

---

## 4. Subject and effective grade

```
effective(event E) = max-by-rank over standing assertions on { E, E's thread, E's patient }
standing          = asserted AND NOT withdrawn
```

Ties break on `content_address` (BYTEA multihash — canonical, collation-free, per ADR-0045/#115). The
tiebreak only decides *which assertion to name as the reason*; the grade itself is order-free.

### Why max, and what it buys

- **Nothing can be forgotten downward.** Inheritance is computed at *read*, so grading a thread covers
  events authored before and after the grading, with no backfill and nobody remembering.
- **It converges for free.** Max is commutative, associative and idempotent — a join-semilattice, i.e.
  a grow-only CRDT. Set-union sync converges on the grade without HLC ordering mattering at all.
- **Uncertainty can only protect.** Every unknown ranks MAX (§5) and combines by max, so doubt
  anywhere in the chain raises the grade. There is no path where confusion lowers protection.

### `subject_kind` is not a closed domain

A future `subject_kind` (say `episode`, once ADR-0020 thin encounters exist) must be **admitted** by an
older node, or the closed CHECK wedges the apply door on honest traffic — ADR-0056: the floor gates
*effect*, not presence.

But an unrecognized `subject_kind` cannot simply contribute nothing: contributing nothing is lowering,
and lowering is a leak. So it is interpreted **conservatively — as chart-wide for the patient named in
its own envelope.** It over-selects, never silently misses (the same discipline `db/006`'s recall
selection already uses), and the envelope bounds the blast radius so it can never reach another chart.

### Chart-wide: expressible, deliberately not easy, never automatic

Whole-chart grading is necessary and cannot be served per-thread: the staff member treated at their own
hospital, the public figure, the DV case where the *fact of any care* is the risk, child protection.
The catastrophic failure there is a *new* thread opened by a clinician who doesn't know — and
patient-wide is the only subject that covers threads nobody has imagined yet.

It is also the one act in this design whose blast radius is the entire record. Once part B lands, a
chart-wide grade coarsens **every** safety signal on that chart, including the ones with nothing
sensitive about them — the metformin interaction and the penicillin allergy blur along with the reason
the grade exists. Two consequences follow, and the second is the serious one:

- **The signal stops carrying information** — if everything on a chart is blurred, blurring
  distinguishes nothing. §5.12's alert-fatigue disease, in the confidentiality dimension.
- **Break-glass fatigue.** The clinician learns that on *this* patient they always have to break glass,
  so they break it reflexively on arrival. That is principle 3's named enemy — the confirmation-dialog
  click-through — reappearing as an audited access event, and it is worse than the dialog: every
  reflexive break-glass writes a record that looks like a deliberate, justified access, so the one that
  mattered becomes indistinguishable from the three hundred that did not.

Three controls, and deliberately **no cap**:

1. **A chart-wide raise requires a `rationale`** (§6) — the single exception to frictionless raising.
2. **The automatic source can never author a chart-wide assertion** (§7). A coded hit on one drug
   tagging the whole chart is precisely "chart-wide as the default for highly sensitive records".
3. **The read surface always names which subject won** (§8) — `restricted (chart-wide)` vs
   `restricted (this thread)`. Without it nobody can tell why a chart is uniformly blurred, and
   therefore nobody can fix it.

Capping chart-wide below `sequestered` was considered and rejected: it would foreclose a legitimate
deployment (whole-chart sequestration is exactly right for a protected-witness case), which is Cairn
taking a policy stance — principle 9 says ship the mechanism. Friction, visibility and
never-automatic are the controls; the ceiling stays open.

### Which subject feeds which dial — named now, decided later

ADR-0006's dial 2 (seal rung / custody) and dial 4 (projection coarseness) both read the grade. This
slice **does not decide that chart-wide grades must drive dial 2**. Part C stays free to let a
chart-wide grade coarsen the safety projection without narrowing custody on every event of the chart —
which is what keeps a graded chart usable. Deciding it now would be premature; foreclosing it would be
the expensive mistake, so it is named here as an open lever.

---

## 5. The ladder, and the inverted unknown

```sql
cairn_sensitivity_rank(g text) RETURNS int IMMUTABLE
    'routine'      -> 0
    'sensitive'    -> 10
    'restricted'   -> 20
    'sequestered'  -> 30
    ELSE           -> 2147483647     -- unknown ranks MAX
```

Open `TEXT`, **no CHECK domain** — a future grade from an upgraded peer is admitted verbatim
(additive-only, principle 11). Gaps of 10 leave room for deployment terms to be interposed later
without renumbering.

**`ELSE` is MAX, deliberately inverting [`cairn_clock_grade_rank`](../../../db/040_clock_confidence_grade.sql)'s
`ELSE 0`.** There, an unrecognized value ranking 0 is safe because rank 0 *withholds reject power*.
Here, an unrecognized value ranking 0 would *withhold protection*: an older node reading a peer's newer
`grade:protected-witness` as "not sensitive" emits an uncoarsened safety projection and renders the
body in the clear — a leak on exactly the events that most needed protecting, in code that looks
correct because it matches the established pattern. **This must be stated in a comment at the
function**, or a future reviewer will "fix" it to match db/040.

The failure mode flips from *leak* (unrecoverable) to *over-coarsening* (honest degradation, repaired
by upgrading the node). Same reasoning as ADR-0006's own *when unsure, err toward essential*.

**Absence is not unknown.** No assertion at all ranks 0 (routine); an unparseable or unrecognized grade
ranks MAX. Collapsing the two would make every event in the record maximally sensitive — principle 4's
*not-yet-asked* vs *unknown*, and the distinction has to survive into the code.

---

## 6. Raising is free; lowering is a ceremony

The asymmetry is the matcher's *false merge ≫ false split* one axis over: **never block a protective
act; always block a protection-removing act until it is accountable.**

| Act | Local authoring door (`db/005`) | Remote apply door (`db/020`) |
|---|---|---|
| Raise, `event`/`thread` | no ceremony — any accountable contributor | admit |
| Raise, `patient` (chart-wide) | **`rationale` required** | admit |
| Withdrawal | **bound human author (ADR-0053) + `rationale` required** | admit |

> **Corrected in implementation — see [ADR-0062](../../spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md) erratum E2.**
> The withdrawal row above is wrong, and this table is where the error started. The `rationale` is a
> **structural** floor (`cairn_check_sensitivity_withdrawal`), dispatched through `cairn_event_twin`
> at **both** doors — so a rationale-less withdrawal is refused remotely too. Only the
> **bound-human-author** half is local-door-only, which makes **two** lenient shapes on the remote
> door, not three. The row is left standing rather than rewritten: this file is the point-in-time
> design record, and ADR-0062 carries the corrected decision.

### The doors are asymmetric on purpose, and doubly so for raises

The ceremony is a **local-authoring rule, never a wire rule** (ADR-0060; the #342 trap). A door check
at apply would let a peer's rationale-less act be refused, forking the event set and wedging
replication. For a *raise* the asymmetry is doubly forced: refusing a peer's protective assertion would
leave our node computing a **lower** grade than the peer's — the refusal is itself a disclosure. Both
directions of that reasoning get a test (the Slice 64 pattern: the asymmetry is tested, not commented).

### Why a bound human author here, when ADR-0061 refused one for registration

ADR-0061 decision 4 rejected gating registration on a bound human author because it blocks *care
documentation*, pushes named patients through the John Doe path, and leaves no forensic record. None of
that applies to a withdrawal: removing protection is an administrative act with a consent basis, not
care documentation, and blocking it delays nothing clinical — the content stays fully readable to
everyone who already has custody; only the *grade* stays high. Stated explicitly so a reviewer sees the
asymmetry was reasoned, not overlooked.

### Withdraw-by-reference

A withdrawal names the specific assertion's `content_address`. That assertion **stays in the log**,
readable and re-assertable — nothing is erased (ADR-0006 decision 3), and the chart history shows the
grade was lowered, by whom, and why.

**Standing is a set difference evaluated at read, never a row deletion at apply.** A withdrawal can
arrive *before* the assertion it withdraws (set-union sync has no ordering), and it must still take
effect when the assertion lands. Same arrival-order independence as ADR-0059's *a strike NULLs the
anchor rather than deleting the row*. So: no FK from withdrawal to assertion, and no delete.

### The ADR-0043 carve-out

ADR-0043 makes agent advisories dismissable by anyone. An auto-applied protective tag is authored by an
advisory actor, so read literally that would let any user silently strip it. **Dismissing a protective
tag is a lowering**, and routes through the ceremony above. Without this carve-out stated, the two ADRs
quietly contradict and the contradiction resolves, silently, in the unsafe direction.

---

## 7. The category blacklist

```sql
CREATE TABLE sensitivity_category_map (   -- EMPTY as shipped
    category TEXT PRIMARY KEY,
    grade    TEXT NOT NULL,
    note     TEXT NOT NULL DEFAULT ''
);

cairn_sensitivity_candidate(p_coded jsonb) RETURNS TABLE (grade text, category text)
```

A pure lookup. **Cairn authors no assertion by itself** — it ships the mechanism, never the list
(ADR-0006 decision 3). All three policies are the same call site with different callers:

- *silent apply* → the caller authors the assertion as a registered advisory actor (`source: advisory`)
- *acceptance required* → the caller shows the candidate; a human authors it
- *manual only* → the caller never calls it

**The candidate's subject is always the event or thread that carried the coded field, never the
patient.** The function cannot express a chart-wide candidate at all — the automatic path must not be
how a chart gets blanket-graded (§4).

Only medication carries coded fields today, so the map has one real consumer; that is honest rather
than a gap, and the lookup is written against a generic `jsonb` of coded fields so the next stream
needs no change here.

---

## 8. Read surface

```
$ cairn-node patient-sensitivity --patient <uuid>
patient <uuid>        effective: sensitive     (chart-wide, a3f9…)
  thread <uuid>       effective: sequestered   (this thread, 91b2…, Dr B, 2026-08-04)
  thread <uuid>       effective: routine       (no assertion)
```

Reports; withholds nothing. **Always names the winning subject kind**, per §4 control 3.

Three CLI verbs: `sensitivity-assert`, `sensitivity-withdraw`, `patient-sensitivity`.

---

## 9. Data model

```sql
CREATE TABLE sensitivity_assertion (
    content_address BYTEA PRIMARY KEY,   -- producing event; the provenance-precise scrub key
    event_id        UUID    NOT NULL,
    patient_id      UUID    NOT NULL,    -- envelope patient: every chart query is one indexed scan
    subject_kind    TEXT    NOT NULL,    -- NO CHECK — see §4
    subject_id      UUID    NOT NULL,
    grade           TEXT    NOT NULL,    -- NO CHECK — open vocabulary
    source          TEXT    NOT NULL,
    rationale       TEXT,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    node_origin     TEXT    NOT NULL
);

CREATE TABLE sensitivity_withdrawal (
    content_address BYTEA PRIMARY KEY,
    event_id        UUID    NOT NULL,
    withdraws       BYTEA   NOT NULL,    -- the withdrawn assertion's content_address; NO FK (§6)
    patient_id      UUID    NOT NULL,
    rationale       TEXT    NOT NULL,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    node_origin     TEXT    NOT NULL
);
CREATE INDEX ON sensitivity_withdrawal (withdraws);
CREATE INDEX ON sensitivity_assertion  (patient_id);
```

`patient_id` on **every** row regardless of subject kind is what keeps the whole computation one
indexed scan per chart — and avoids repeating [#336](https://github.com/cairn-ehr/cairn-ehr/issues/336)
(the med-list read path is O(all medications on the node) per chart open).

### Functions

| Function | Purpose |
|---|---|
| `cairn_sensitivity_rank(text) -> int` | the ladder (§5), pure/IMMUTABLE |
| `cairn_sensitivity_standing(uuid) -> setof` | one definition of *standing* for a chart: assertions minus withdrawals |
| `cairn_event_thread(uuid) -> uuid` | E → thread; **NULL when unresolvable** (§10) |
| `cairn_effective_sensitivity(uuid) -> (grade, subject_kind, content_address)` | the reader's entry point |
| `cairn_sensitivity_candidate(jsonb) -> (grade, category)` | the blacklist lookup (§7) |

---

## 10. The two leaks, and their guards

### 10a. `cairn-sync`'s SCHEMA subset

`cairn-sync` loads an **explicit subset** of the migrations
([`crates/cairn-sync/src/main.rs`](../../../crates/cairn-sync/src/main.rs) `const SCHEMA`) — it carries
db/045 and db/047 but not db/041/042/046. If `db/048` is omitted, a node syncing through `cairn-sync`
stores the assertion in `event_log` and has **no `sensitivity_assertion` row**, so
`cairn_effective_sensitivity` returns *routine* and the body renders in the clear.

Slice 64's lesson 2 — *when you add a rule to a shared file, re-check every subset that loads it* —
except here the failure is disclosure, not a wedged door. **db/048 goes in both loaders**, and
`cairn-sync`'s existing `schema_subset_satisfies_its_own_doors` test is what proves it by failing
first.

### 10b. Thread resolution needs custody, so the grade computes *lower* where custody is thinner

`medication_id` lives **inside the sealed payload**
([`db/031_medication.sql`](../../../db/031_medication.sql)) and `event_log` carries no thread column in
the clear. `medication_statement` is populated through `cairn_clear_payload`, so on a node holding no
custody the row is **absent**, E → thread resolution fails, the thread's grade does not apply, and the
effective grade comes out lower — on precisely the node least entitled to see anything.

The fix follows §5's direction rather than adding machinery, but tightly:

```
thread contribution =
    resolved T                        -> max over standing thread-assertions on T
    unresolved, chart HAS thread-assertions
                                      -> max over ALL standing thread-assertions on this chart
    unresolved, chart has none        -> no contribution
```

The middle rule is a **precise conservative bound**, not a sentinel: an unresolvable event belongs to
*some* thread on this chart, so the tightest safe answer is the max over the chart's thread grades. It
needs no artificial MAX value, and the third rule keeps the uncertainty from biting where it cannot
matter — without it, every medication event on every custody-less node would coarsen maximally,
recreating the everything-is-blurred problem in §4.

**Where the cost actually lands.** Under sequester the unresolvable set is *exactly* the sequestered
set, so a node that holds custody of the chart's other threads coarsens nothing extra. The cost falls
on a node with **no** custody at all — a phone-tier node that has synced a chart but fetched no DEKs —
where a chart with one graded thread and twenty ungraded ones coarsens all twenty-one. Two things bound
that: such a node cannot render any body anyway, so the blurred projection is all it was ever going to
show; and the warning still fires, so the §5.9 safety floor holds. The damage is friction and
break-glass pressure, never a missed signal.

**This rule is required by §5.9 today, not only by part C.** When an event is crypto-shredded (rung 3),
`db/037` scrubs its derived projection rows, so `cairn_event_thread` returns NULL for it **on every node
permanently** — including the authoring one. Event-scoped and chart-scoped assertions still apply (they
need no resolution); only thread membership is lost. Without the bound, a shredded event's thread grade
evaporates at the moment of shred and its safety projection renders uncoarsened — contradicting §5.9's
*"the safety projection outlives the body it protects — coarsens but survives."* So the bound is what
makes coarsen-but-survive true after a shred, independent of sequester.

**Consequence: effective grade is non-monotone in custody.** Gaining custody can *lower* a displayed
grade, as the bound collapses to the true value — so the grade is a function of **local custody, not a
global fact**. This is a known pattern here rather than a surprise: ADR-0052 §9 found the same thing
about ADR-0049's thread commitment, which born-sealing turned from a pure function of the content-event
*set* into a function of local *custody*. Two consequences: a UI showing a grade must tolerate it
dropping as DEKs arrive, and the §12 convergence test is only valid **given equal custody** (below).

It opens **no new inference channel**: the bound reveals the chart's highest thread grade to a node that
can resolve nothing, but the assertions are plaintext and replicate unconditionally (§3), so that grade
was already readable there.

**Rejected alternative — a plaintext thread reference on `event_log`**, which would make resolution
custody-free and delete this whole rule. It fails on its own terms: *"these eight events form one
thread"* is itself linkage information, and thread size and timing re-identify — so it trades a
coarsening cost for a disclosure. It is also an envelope wire change, which ADR-0052 §2 scopes
deliberately. Named here so the next reader sees it was weighed.

### 10c. Recall marks; it must never lower

`recall_overlay` is append-only, node-local, and **consulted by nothing outside `db/006`** — it marks
affected events, it does not suppress them. So a recalled actor's sensitivity assertion stays standing
today, which is the safe direction by accident rather than by design.

Wiring recall into projections is a plausible future change, and if a sensitivity assertion authored by
a since-recalled actor fell out of the standing set, **recalling a bad actor would silently strip
protection from every patient they graded.** A test pins the safe behaviour now, so that change cannot
land quietly. (Slice 64 already showed the cascade reaches further than it looks — a chart's
registration turned out to be in its author's recall set.)

---

## 11. Paper-parity benchmark (§1.2, CLAUDE.md house rule 7)

**Paper counterpart:** the *confidential* sticker, or the sealed envelope clipped inside a paper file.

| Workflow | Paper N | Architecture-forced M | UI target K |
|---|---|---|---|
| Grade a thread | 2 (retrieve file, affix marker) | **1** (one assertion event) | 1 |
| Grade a whole chart | 2 | **1** (assertion + rationale in one act) | 1 |
| Declassify | 3 (remove marker, initial, note why) | **1** (one withdrawal event) | 2 |

`M > N` nowhere. Declassification's **K=2** (rationale + act) is the same price `medication-cease`
charges, for the same ADR-0060 reason — a cancellation carries an owner *and* a rationale — and K=2
still beats paper's N=3.

**Time budget:** ≤ 10 s to grade a thread, ≤ 20 s to declassify (the rationale is the cost). This
slice's only surface is a CLI, so per the §1.2 rule the **interactive measurement is owed by the first
UI surface**, not here.

The node-tier write cost is **not measurable yet either**, for the same reason registration's is not:
`db/044`'s `ui_gesture_timing_kind_ck` is `CHECK (gesture_kind IN ('signoff','cease'))`, so it refuses
a sensitivity row exactly as it refuses a registration row
([#360](https://github.com/cairn-ehr/cairn-ehr/issues/360)). This slice does **not** widen it — #360
already owns that widening, and doing it twice invites two half-widenings that disagree. The
sensitivity gesture kinds are added to #360's scope instead, and the write-cost figure lands with it.
Expected magnitude, for whoever runs it: near the 222 ms sign-off figure (one event, one projection).

---

## 12. Test plan (TDD — every test written red first)

**Pure (`cairn-event`, no DB):** body builders round-trip; `subject_kind`/`grade` are open strings.
The builder **must be able to construct a rationale-less chart-wide raise and a rationale-less
withdrawal** — the remote-door leniency tests need exactly those bodies, so rationale is a *door* rule
(§6), never a builder invariant. A builder that refused them would make the asymmetry untestable.

**Ladder:** named ordering; `routine` = 0; **unrecognized = MAX**; absence ≠ unknown.

**Effective grade:** event-only; thread inheritance reaches an event authored *after* the grading;
chart-wide; max across all three; the winning `subject_kind` is reported.

**Withdrawal:** removes from standing; **arrival-order independence** (withdrawal applied before its
assertion still takes effect when the assertion lands); re-assertion after withdrawal restores the
grade; the withdrawn assertion is still in `event_log`.

**Conservative interpretation:** unrecognized `subject_kind` is treated chart-wide **and never crosses
into another chart**; unresolvable thread + chart has thread-assertions → the bounded max;
unresolvable + none → no contribution.

**Door asymmetry (Slice 64 pattern — tested, not commented):** local door refuses a chart-wide raise
with no rationale, a withdrawal with no rationale, and a withdrawal with no bound human author, each
with `P0001`; the **remote door admits all three**.

**Hex legibility (#228 contract):** a malformed `withdraws` fails through
`cairn_decode_hex_or_raise(...)` naming field and door, and raises **`P0001`** — asserted on the
SQLSTATE, not the message, because a message-only assertion stays green through a well-meaning
`USING ERRCODE = SQLSTATE`.

**Leak guards:** `cairn-sync`'s SCHEMA subset carries db/048; recall marks but never lowers (§10c).

**Blacklist:** empty map yields no candidate; a populated map yields grade + category; the function
authors nothing; **no input can produce a chart-wide candidate**.

**Convergence, *given equal custody*:** two nodes receiving the assertions in opposite orders compute
the same effective grade (the CRDT property of §4). The custody qualifier is load-bearing and belongs
in the test name — §10b makes the effective grade non-monotone in custody, so two honest nodes with
*different* custody may legitimately disagree. Stated loosely this test either fails spuriously or, far
worse, gets "fixed" by deleting the §10b bound — reopening the leak it exists to close.

**Registration precedence (Slice 64):** a sensitivity assertion bears `patient_id`, so it cannot be a
chart's first event — fixtures register first, and one test pins the refusal.

Each DB-gated Rust test gets its `db/tests/048_sensitivity_stream_test.sql` mirror.

---

## 13. Files, and the repo's standing traps

| File | Change |
|---|---|
| `db/048_sensitivity_stream.sql` | new — tables, functions, floor checks, registry rows |
| `db/tests/048_sensitivity_stream_test.sql` | new — SQL mirror (opens with the `_scratch_database_guard.sql` include, per #169) |
| `crates/cairn-event/src/sensitivity.rs` | new — pure builders |
| `crates/cairn-event/src/schema_generation.rs` | 47 → 48 |
| `crates/cairn-node/src/db.rs` | SCHEMA list + db/048 |
| **`crates/cairn-sync/src/main.rs`** | **SCHEMA subset + db/048 — §10a** |
| `crates/cairn-node/src/sensitivity.rs` + CLI | orchestrator + three verbs |
| `crates/cairn-node/tests/twin_registry.rs` | **+2** |
| `db/tests/034_twin_registry_test.sql` | **+2 — the count lives in BOTH places** |
| `docs/spec/decisions/0062-*.md` | new ADR |
| `docs/spec/identity.md` §5.9, `docs/spec/index.md` | prose + v0.64 |

Standing traps this slice walks straight into, all recorded from earlier sessions:

- **Guard before connect** — `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
- **UUIDs bind as text** — bind `&uuid.to_string()`, cast `$1::text::uuid`.
- **Registry rows use `ON CONFLICT … DO UPDATE`**, not `DO NOTHING` (#214), so replay converges and
  heal mode can re-derive them (#277).
- **`event_type_class` for both types is `('additive', false)`** — a withdrawal is cross-author *by
  design* (ADR-0006 requires authority-based declassification, not ADR-0043 self-only), so it must not
  route through the self-only suppression gate. The ceremony in §6 is the substitute control, and the
  migration says so in a comment.
- **Full-workspace `cargo test`**, never `-p cairn-node` alone — the `cairn-sync` change is cross-crate,
  and per-crate runs miss exactly that.

---

## 14. Follow-ons to file

- **B** — safety-projection emission (carries #294: the class is captured pre-seal and *carried*, never
  re-derived).
- **C** — sequester / custody narrowing. **Blocked on #231.**
- **D** — break-glass: audited key-use, partition-honest disclosure.
- **Sealed-rationale variant** — a withdrawal rationale is clear text forever and replicates (§3).
- **Grade in the legibility twin** — the twin should render the effective grade, mirroring
  [#283](https://github.com/cairn-ehr/cairn-ehr/issues/283) for `clock_grade`.
- **Which subject feeds which dial** — the §4 lever part C must decide.
