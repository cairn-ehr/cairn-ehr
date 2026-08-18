# Design — the §5.9 operator surface tells the whole truth

**Status:** approved 2026-08-18 · **Issues:** [#388](https://github.com/cairn-ehr/cairn-ehr/issues/388)
(all four parts), [#383](https://github.com/cairn-ehr/cairn-ehr/issues/383) (the same defect as #388
part 3, filed separately), [#421](https://github.com/cairn-ehr/cairn-ehr/issues/421) (folded in — one
line) · **Discharges:** [ADR-0064](../../spec/decisions/0064-admit-the-claim-withhold-the-power.md)'s
§1.2 budget, recorded there as *owed, not met* · **ADR:** none — this takes no new architectural
decision · **`SCHEMA_GENERATION`:** stays 49 (no new `db/NNN` file) · **Spec version:** unchanged.

---

## 1. What is owed, and by whom

Three slices in a row shipped a §5.9 mechanism and no way to look at it.

- **Slice 65** (ADR-0062) built the sensitivity stream. `patient-sensitivity` reports the effective
  grade — honestly, but silent about three states an operator most needs to see (#388, #383).
- **Slice 67** (ADR-0063) sealed a precise `{class, severity}` with the body. Nothing reads it
  ([#407](https://github.com/cairn-ehr/cairn-ehr/issues/407)).
- **Slice 68** (ADR-0064) added `sensitivity_withdrawal_worklist` and `safety_overclaim_flag`. Both are
  tested, both carry `GRANT SELECT … TO cairn_agent`, and **nothing in the workspace displays either.**

ADR-0064 states the cost in its own §1.2 section: an operator must be able to answer *"why did this
withdrawal not take effect?"* in **one query with no raw SQL**, and records that budget as **owed, not
met**. That sentence is the specification this design implements. Everything else here is the same
defect in its other four shapes.

**The pattern is worth naming, because it is what makes this a slice rather than a chore.** A
mechanism whose whole safety story is *honest degradation* degrades honestly only if someone can see
it degrade. An unread surface is not a partially-delivered feature; it is a mechanism whose failure
mode is silence — which is the failure mode §5.9 exists to prevent.

## 2. What already exists, and is unwired

| Surface | Home | Granted to | Read by |
|---|---|---|---|
| `sensitivity_withdrawal_worklist` (view) | `db/048` §11 | `cairn_agent` | nothing |
| `safety_overclaim_flag` (ledger) | `db/049` §6 | `cairn_agent` | nothing |
| `event_deferred.adjudication_error` | `db/001` | **`cairn_node` only** | `cairn-node deferred`, node-wide |
| `cairn_sensitivity_standing(patient)` | `db/048` | `cairn_agent` | `chart_sensitivity`'s no-registration fallback only |

The worklist already answers the §1.2 question precisely: its `reason` column is `inert` (no accountable
human stands behind the claim) or `stranger-attested` (attested, but by someone with no prior presence
on this chart), and it carries the withdrawal's own `rationale`. **The answer exists; the query is
missing.**

`event_deferred` is the one that constrains the design — see decision 5.

## 3. Decisions this design takes

**1. One verb, not two.** The budget is *one query*, so the surface must be the query an operator
already runs. Every anomaly folds into `patient-sensitivity <chart>`; no second verb to know about. A
separate `sensitivity-worklist` verb would satisfy the letter of "no raw SQL" and fail the budget,
because an operator who does not know to run it is exactly the operator the budget describes.

**2. Name, never count.** Both #388 part 3 and #383 propose a *count* (`2 standing assertion(s) not
accounted for above`). This design **diverges from both, deliberately**, and names each assertion —
grade, subject kind, `content_address`. [ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)
settled this shape for the registration funnel: the attestation *names* the displayed candidates
because *"was the duplicate on screen?"* has opposite fixes for yes and no, and `N = 3` cannot separate
them. The same holds here. *"3 standing assertions, 0 threads shown"* cannot tell an operator whether
this node is custody-blind or the chart genuinely has no medications — which is the single question
part 3 exists to answer. A count also asks the reader to reconcile two numbers mentally; a named row
carries its own `content_address`, which is what `sensitivity-withdraw --withdraws` consumes.

**3. An anomaly is loud, and adjacent to the grade it contradicts.** The grade line stays first — it
is one line, it is what was asked for, and its printed `content_address` is a documented contract (see
decision 6). The anomaly block sits immediately beneath it, before the thread breakdown, because an
inert withdrawal means *the grade above may not be what someone intended*, and a warning that appears
forty thread-lines below the claim it qualifies is a warning nobody reads. This is the med-list slice's
lesson (*a unit-tested safety control can still be defeated by the surface that calls it*) applied to a
read path.

**4. The report states its own blind spots.** ADR-0064's *Known limitations* records that a cross-chart
mis-targeted withdrawal which stays `unverified` is **permanently inert and permanently invisible** —
it falls out of the worklist's `inert` arm, because that arm asks whether the target still stands on
the withdrawal's *own* chart, where it never did. And
[#414](https://github.com/cairn-ehr/cairn-ehr/issues/414) records that the overclaim ledger's
completeness rests on a `RAISE WARNING` nothing consumes, so an empty ledger is indistinguishable from
a broken one. A surface that lists "the withdrawals that did not take effect" while silent about either
is a **comment asserting a guarantee the code does not provide** — the largest single defect class of
the last three sessions, and the one this project keeps re-committing while fixing it. Both are printed
in the footer.

**5. A chart-scoped definer, not a table grant.** `event_deferred` is granted to `cairn_node`, not
`cairn_agent`. Reading it directly works *today* only because the runtime login role happens to be a
`cairn_node` member — which is precisely
[#425](https://github.com/cairn-ehr/cairn-ehr/issues/425)'s finding, and building on it would bake a
known-unreliable membership into a new read path. The alternative of granting `cairn_agent` the whole
table widens the role's reach node-wide to answer a chart-scoped question. So: a `SECURITY DEFINER`
function scoped to one chart, following the precedent db/049 set when `cairn_event_safety` became a
definer for the same reason.

**6. Rendering is pure; reading is separate.** The report's honesty claims are all *wording* — "this is
not a clean bill of health", "this node may hold no custody", "this list is not complete". Those belong
in pure functions over a plain struct, unit-tested with no database, following the precedent
`safety::render_safety_line` already sets. Today the wording lives in `println!` calls inside
`main.rs`'s match arm, where it can only be tested by running the binary against a live cluster — which
is why nobody has.

**7. The read-back prints two facts, not one.** #388 part 4 asks `assert_sensitivity` to confirm the
grade took effect. Re-reading `cairn_effective_sensitivity` and printing one grade would mislead: a
thread-scoped `restricted` asserted under a standing chart-wide `sequestered` reads back as
`sequestered` — correct, and indistinguishable from *"your assertion was silently upgraded"*. The
surface prints **what you asserted** and **what now stands, with its winning subject**, as two distinct
facts.

## 4. Where it lives

`crates/cairn-node/src/sensitivity.rs` (411 lines, and this adds four read paths) becomes a module
directory. House rule 4 is the trigger; the testability in decision 6 is the reason the split falls
where it does.

| File | Contents |
|---|---|
| `sensitivity/mod.rs` | `assert_sensitivity`, `withdraw_sensitivity`, `subject_kind_phrase`; re-exports so `cairn_node::sensitivity::…` paths at every call site are unchanged |
| `sensitivity/report.rs` | `ChartReport` and its row structs; `chart_sensitivity()`; the four new reads. The **only** DB-touching file |
| `sensitivity/render.rs` | `render_chart_report(&ChartReport) -> Vec<String>` and its helpers. **Pure** — no `tokio_postgres` import |

`main.rs`'s `Cmd::PatientSensitivity` arm shrinks to *connect → read → print each line*, losing ~30
lines of `println!` rather than gaining any. `Cmd::SensitivityAssert` gains the decision-7 read-back.

## 5. The SQL — two in-place edits, no new migration

`SCHEMA_GENERATION` is pinned to the newest migration **filename**, so editing home files leaves it at
49. `connect_and_load_schema` re-runs every `db/*.sql` on each connect, so an in-place `CREATE OR
REPLACE` in the file that owns the object *is* the repo's change idiom — and it avoids the
view-widening-across-files trap (#207), which is what a new `db/050` re-declaring db/048's view would
be.

**`db/048` — project `responsible_actor_id` (closes #421).** The `judged` CTE already computes it (the
vouched R1 attester, or the withdrawal's own actor); the outer `SELECT` then drops it. Adding it to
the projection is one line. It is folded in here rather than left open because a worklist that says
*"this withdrawal did not take effect"* without naming **who tried** is half a report — and it is the
field #421 says the row exists to report. `CREATE OR REPLACE VIEW` permits appending a column at the
end of the list, so the change is additive for any existing reader.

**`db/043` — `cairn_patient_deferred_sensitivity(uuid)`.** A `SECURITY DEFINER` reader returning this
chart's deferred `sensitivity.%` events (`event_id`, `event_type`, `admitted_at`,
`adjudication_error`), joining `event_deferred` to `event_log` for the `patient_id`. `search_path`
pinned with **`pg_temp` last** (#426 — a definer reading `event_log` unqualified is exactly the shape
that was blindable), `REVOKE EXECUTE … FROM PUBLIC` then `GRANT … TO cairn_agent` (#382's posture,
applied on the way in rather than retrofitted).

## 6. What the report prints

The grade line keeps its **exact current shape**. `sensitivity-withdraw --withdraws` documents its
argument as *"the hex content_address, as `patient-sensitivity` prints it"* — a documented contract,
and the struct doc records that an earlier draft broke it and a hand exercise of the CLI caught it.

```
chart <uuid>: sequestered (winning subject: chart-wide, withdraws=a3f…)
⚠ 2 withdrawals on this chart did NOT take effect — the grade above may not be what someone intended
    inert              withdraws=a3f…  by actor=<id>  origin=peer-b
      rationale: "consent withdrawn by patient 2026-08-12"
      → no accountable human this node can hold responsible stands behind it (ADR-0064)
    stranger-attested  withdraws=b71…  by actor=<id>  origin=peer-c
      rationale: "…"
      → attested, but by an actor with no prior presence on this chart
⚠ 1 sensitivity event on this chart is DEFERRED — admitted, powerless, not applied
    <event_id>  sensitivity.grade.asserted  (not yet re-adjudicated)
⚠ safety overclaim recorded for 1 event on this chart: emitted=precise licensed=existence
  thread <uuid>: restricted (winning subject: this thread, withdraws=c02…)
```

**The custody-blind branch.** The line `no medication threads on this chart` is a precise untruth on a
node with no DEK custody (#383). It is replaced by whichever of these is true:

- nothing projected **and** nothing standing → `no medication threads and no standing sensitivity
  assertions on this chart`
- nothing projected **but** assertions stand → each **named**, followed by: `this node projects no
  medication threads — it may hold no DEK custody, so the threads these assertions grade may exist and
  be invisible here`

**The footer** keeps the existing *report only — nothing is withheld; enforcement needs custody
narrowing (#232 part C / #376)* line, and adds decision 4's two blind spots.

## 7. What this does not fix, declared

- **[#387](https://github.com/cairn-ehr/cairn-ehr/issues/387)** (a `Provenance` enum, ladder constants,
  a sum type for the report's correlated `Option` pair) touches these exact structs and stays open. It
  is a type-design change with its own review surface; doing it inside a slice that is already moving
  three files would make both harder to review.
- **[#414](https://github.com/cairn-ehr/cairn-ehr/issues/414)** — the ledger's completeness is not
  fixed, only *declared* in the footer. Fixing it means giving the `RAISE WARNING` a consumer, which is
  a different mechanism.
- **ADR-0064's permanently-invisible cross-chart withdrawal** is not made visible. It cannot be, from
  the worklist; it is declared instead.
- **No node-wide sweep.** `patient-sensitivity` stays chart-scoped. The views are node-wide and a hub
  tier will eventually want the sweep; that is a separate issue when someone wants it.
- **Nothing is enforced.** This slice withholds no content on the strength of any grade. Enforcement is
  custody narrowing (#376), and a projection-layer filter with no floor beneath it is security theatre
  — the module doc's existing position, unchanged.

## 8. Paper-parity benchmark (§1.2)

This changes a clinical-adjacent operator workflow (reading a chart's confidentiality state), so it
carries the benchmark rather than the forced-rationale escape.

- **Paper counterpart:** reading a paper chart's confidentiality state — the restriction sticker on the
  cover, *and* any struck-and-initialled removals, both visible in one glance at the same cover. Paper
  does not make you turn to a second document to learn that a restriction was struck.
- **Paper *N* = 1** human act (look at the cover).
- **Architecture-forced *M* = 1.** One invocation of `patient-sensitivity <chart>` returns the grade
  and every anomaly this node can see. **M = N — no architecture defect.**
- **UI bundling target *K* = 1** — unchanged; there is one command.
- **Time / cognitive load budget:** ADR-0064's own, restated falsifiably — an operator must be able to
  answer *"why did this withdrawal not take effect?"* in **one query with no raw SQL**. **Measurement:**
  run `patient-sensitivity <chart>` on a chart carrying an inert withdrawal; the `reason` and the
  withdrawal's `rationale` must both be on screen, and the accountable actor named. This is pinned by a
  DB-gated test rather than left to a hand exercise, so the budget is met *and* kept met.
- The node-tier read cost is measurable through the existing
  [`RUNBOOK`](../../../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md); the **interactive** half stays
  owed by the first GUI surface ([#400](https://github.com/cairn-ehr/cairn-ehr/issues/400)) and is not
  claimed here.

## 9. Test plan (TDD — every test red first)

**Pure, no database** (`sensitivity/render.rs`, `#[cfg(test)]`):

1. A healthy chart prints **no** `⚠` line.
2. An inert withdrawal prints its reason, its rationale, its accountable actor, and the ADR-0064
   explanation.
3. A `stranger-attested` withdrawal prints its own distinct explanation, not the inert one.
4. Nothing projected **and** nothing standing → the both-empty wording; the old
   `no medication threads on this chart` string appears nowhere in the crate.
5. Nothing projected **but** assertions standing → each named with grade, subject kind and
   `content_address`; the custody sentence present; **no bare count**.
6. The footer carries both blind-spot sentences whether or not the corresponding lists are empty (the
   #414 case is the one that matters: an *empty* ledger must still disclaim).
7. The grade line's byte shape is unchanged for a chart with no anomalies (the `--withdraws` contract).

**DB-gated** (`crates/cairn-node/tests/sensitivity_report.rs`):

8. An un-attested strip surfaces with `reason = 'inert'` — needs two **distinct** human actors, so
   `enroll_human_with_role`, not `enroll_human` twice (which collides on the pinned determinant set and
   is refused, ADR-0044/#152).
9. A withdrawal attested by an actor with no prior presence surfaces as `stranger-attested`.
10. A deferred `sensitivity.%` event on the chart surfaces; one on a **different** chart does not (the
    definer really is chart-scoped).
11. A chart with standing assertions and no projected `medication_statement` rows names them.
12. `safety_overclaim_flag` rows for the chart surface; rows for another chart do not.
13. The §1.2 budget itself: one call to `chart_sensitivity` yields both the reason and the rationale.

**Guard:** `responsible_actor_id` is projected by the view (a `SELECT` naming the column, so dropping
it from db/048 fails loudly rather than silently emptying a report field).

## 10. Files, and the repo's standing traps

**Touched:** `db/048_sensitivity_stream.sql` (one projected column), `db/043_deferred_readjudication.sql`
(one definer), `crates/cairn-node/src/sensitivity.rs` → `sensitivity/{mod,report,render}.rs`,
`crates/cairn-node/src/main.rs` (two match arms), `crates/cairn-node/tests/sensitivity_report.rs` (new).

**Traps this slice must clear, from HANDOVER and the memory of prior sessions:**

- **UUIDs bind as text** — `cairn-node` has no `with-uuid-1`, so bind `&uuid.to_string()` and cast
  `$1::text::uuid`.
- **Guard before connect** — `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
- **`content_address IS NOT NULL` is the "did anything win" test**, never `subject_kind <> 'none'`
  (ADR-0062 E6 — `none` is a legal open-vocabulary value that collided with the sentinel).
- **`pg_temp` last** on the new definer's `search_path` (#426). Verified: `search_path_pg_temp.rs`
  compares with `rows.len() >= PINNED_TODAY` (25), **not** `==`, and its header says so deliberately
  (*"the count only ever grows"*). A twenty-sixth pinned definer therefore needs **no** number moved —
  but it must genuinely carry the clause, because the same file asserts that *every* `SECURITY
  DEFINER` pins a path and that every pinned path denies the temp schema on the first look.
- **`db/tests/048` pins the worklist's column set, order AND types** (its blocks around lines
  315–348, via `information_schema`). Adding `responsible_actor_id` **will** fail that guard, by
  design — the pin must be updated in the same commit, and updating it is the deliberate act that
  makes the new column reviewed rather than absorbed.
- **Both changed files are already in `cairn-sync`'s migration list** (db/043 at `main.rs:143`, db/048
  at `:167`), so neither edit opens the subset gap that bit Slice 64. Verified rather than assumed.
- No new event type and no new projection, so the **four registry row-counts** (`twin_registry.rs`,
  `db/tests/034`, `projection_registry.rs`, `db/tests/039`) are untouched. Stated so the next reader
  does not go looking.

## 11. Follow-ons to file

Anything found during the build that is out of scope goes to an issue rather than into the diff (house
rule 5). Expected candidates: whether a hub tier wants the node-wide sweep; whether #387's type work
should precede or follow this; and whether the `deferred` verb and this report should share one
rendering path.
