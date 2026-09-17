# Design — the two states where a restore still exits 0 having left a record behind (#614, #615)

- **Issues:** [#615](https://github.com/cairn-ehr/cairn-ehr/issues/615) — `restore_node_event` lacks the
  substitution guard `submit_event` and `apply_remote_event` both have ·
  [#614](https://github.com/cairn-ehr/cairn-ehr/issues/614) — a newer Cairn's clinical event TYPE is
  admitted deferred and exits 0 in silence, while a newer PLANE exits 3. Both also narrow
  [#608](https://github.com/cairn-ehr/cairn-ehr/issues/608) (both substitution guards fail OPEN on a
  NULL comparison).
- **Date:** 2026-09-17. **Branch:** `feat/614-615-restore-loses-no-record-silently`.
- **Records:** a new **ADR-0072**, continuing [ADR-0071](../../spec/decisions/0071-a-restore-that-left-records-behind-exits-incomplete.md).
  New migration `db/053_substitution_guard.sql`; **`SCHEMA_GENERATION` 52 → 53**.

## 0. Maintainer decisions taken in the brainstorm (2026-09-17)

Three forks were put to the maintainer and ruled on before any code was written. They are recorded
here because each one closes off an approach a later session would otherwise re-open in good faith.

| # | Fork | Ruling |
|---|------|--------|
| 1 | Port db/005's guard verbatim (copying #608's fail-open), or extract a shared helper? | **Shared helper, all three doors.** #608's guard half is fixed once; its second half (`cairn_project_late_custody`'s silent not-found arm) is explicitly **not** in scope, so #608 narrows rather than closes. |
| 2 | Helper in `db/001` beside its closest sibling (no generation bump), or a new `db/053` (bump to 53)? | **New `db/053`, bump to 53** — so the #188 downgrade guard covers the change. Placement is subordinate to that. |
| 3 | Does a deferred record become a sixth `Unrestored` cause (exit 3), or a reported count (exit 0)? | **Reported count, exit 0** — the maintainer's own reading in #614. The record IS in the log; ADR-0071's rule is satisfied; the *silence* is the defect, not the verdict. |

A fourth question was **not** asked, because db/009 already answers it: a substitution refusal aborts
the whole restore (`apply_medium` propagates with `?`). See §2.3.

## 1. Why this piece exists

ADR-0071 published exit **0** as a contract: *every record the medium carried is in this node's log*.
Publishing a contract is what turns every remaining way to reach 0 dishonestly into a defect worth
naming — the reasoning that filed #613, #614 and #615 within hours of the ADR being written.

Two of those states let a restore reach exit 0 **having left a record behind**. They are unrelated in
mechanism and share one consequence, which is why they travel together:

| | #615 | #614 |
|---|------|------|
| Plane | node (trust set) | clinical |
| Mechanism | a second event reusing an `event_id` is silently discarded by `ON CONFLICT DO NOTHING` | an event whose TYPE this build cannot classify is admitted *deferred* |
| What the operator reads | `restored N event(s)`, exit 0 | `N applied … (of N on the medium)`, exit 0 |
| What is true | one of two rival events is in the log and nothing says which | the record is in the log, projects into no chart, and appears in no summary |
| Remedy today | none — nobody knows | upgrade the node; `connect_and_load_schema` re-adjudicates |
| Severity | **security** — a dropped `peer.revoked` restores a node trusting a revoked peer | **legibility** — the charts are silently short until someone notices |

### 1.1 #615 in detail

Two of the three write doors refuse a content substitution loudly:

- `db/005_submit.sql:1505-1512` — `GET DIAGNOSTICS v_log_rows = ROW_COUNT;` then
  `RAISE EXCEPTION 'submit_event: event_id % already exists with different content (substitution refused)'`
- `db/020_apply_remote_event.sql:468-480` — the same guard, same message shape.

The third does not. `db/009_node_supersede_and_restore.sql:99` and `:137` are
`INSERT … ON CONFLICT (node_event_id) DO NOTHING` with **no** `GET DIAGNOSTICS` and **no**
comparison. Two distinct node events sharing one `event_id`: the first wins, the second is silently
discarded, nothing reports it.

The count cannot catch it. `apply_medium` returns `Ok(events.len())` — documented as *"the number of
events PROCESSED … not the number newly inserted"* — so `restored {applied} event(s)` counts what was
**offered**. `events.len() == counts.node` by construction; comparing them would prove nothing. The
guard is the load-bearing half.

**Reachability is not theoretical.** db/009's own comment at `:68-77` already argues it, to justify
the HLC drift ceiling it added: the restore door is **self-trusting** (any validly-signed
`node.enrolled` is admitted without a trust check) and *"the medium can contain OTHER signers' events
and is attacker-appendable."* An attacker with write access to the sneakernet medium appends an event
carrying the `event_id` of the victim's `peer.revoked`, positioned earlier in file order. The genuine
revocation is dropped; the restored node comes back **trusting a peer the clinic had revoked**; the
summary reads `restored N event(s)`; exit 0. The benign variants — a UUID generator bug, a hand-merged
medium — produce identical silence.

### 1.2 #614 in detail

1. `apply_remote_event` (db/020:211-213) looks up `event_type_class` and sets `v_deferred := (v_mode IS NULL)`.
2. It admits the event uninterpreted — db/020's own words: *"It yields NO projection rows and confers
   NO power"* — and returns `Ok`.
3. `apply_clinical_plane` (`crates/cairn-node/src/restore/clinical.rs:371`) sees `Ok(_)`; custody is
   orthogonal to classification so `custody_landed` passes; `report.applied += 1`.
4. The summary prints `clinical records: N applied, 0 already present, 0 refused (of N on the medium)`.
5. Every `Unrestored` field is zero, `notice()` is `None`, **exit 0**.

`restore/clinical.rs` contains no mention of `deferred` and the summary emits nothing about it.

**Why it matters more than the unroutable-plane case, which ADR-0071 gave exit 3.** Per ADR-0012,
additive schema evolution means **a new clinical event type is the case that actually happens**; a
whole new plane is rare. The realistic DR box — a spare laptop one release behind the live node — hits
this and not the plane case. Same underlying cause (the medium was written by a newer Cairn), same
remedy (upgrade), opposite report.

## 2. The design

### 2.1 One shared refusal, three doors (`db/053_substitution_guard.sql`)

```sql
CREATE OR REPLACE FUNCTION cairn_refuse_substitution(
    p_found_ca BYTEA, p_new_ca BYTEA, p_event_id UUID, p_door TEXT
) RETURNS VOID
```

It raises when `p_found_ca IS DISTINCT FROM p_new_ca`, and does nothing otherwise.

**`IS DISTINCT FROM`, not `<>`** — this is #608's guard half. The existing guards are reached only when
the INSERT was a no-op, i.e. when a row with that id exists; if the sub-select nonetheless returns no
row, `<>` yields NULL, the `IF` does not fire, and **the guard passes silently**. That state should be
unreachable (under READ COMMITTED the next statement's snapshot sees the committed conflicting row;
under REPEATABLE READ or SERIALIZABLE the `ON CONFLICT DO NOTHING` raises 40001 first) — so this is
hardening, not a live bug. But writing a **third** copy of a known fail-open is not a defensible way to
fix a door, and three doors spelling one invariant two ways is the drift #159 needed a byte-identical
source guard to catch.

**Pure — it reads no table.** Both addresses arrive as arguments. That is what lets it serve `event_log`
(db/005, db/020) and `node_event` (db/009) without knowing about either. It therefore belongs to none of
the four `REVOKE EXECUTE … FROM PUBLIC` families in `floor_execute_grants.rs`, exactly as its closest
sibling `cairn_decode_hex_or_raise` (db/001) does not — a pure raiser that reads nothing and writes
nothing teaches a caller strictly less than the door already tells it by refusing. **That must be stated
in a comment**, because #382's whole point is that a missing `REVOKE` which a reader cannot classify as
deliberate is worse than either extreme, and #609 is the same omission one slice earlier.

**Message preservation.** `RAISE EXCEPTION '%: event_id % already exists with different content
(substitution refused)', p_door, p_event_id` reproduces both existing strings byte-for-byte. No existing
test's expected text moves; that is a property to assert, not to assume.

### 2.2 db/005 and db/020 — structure kept, comparison swapped

Both stay on `GET DIAGNOSTICS` + `IF v_rows = 0`. They are on the clinical hot path — the measured
restore is 100 003 events at 1.17 ms/event — and must not gain a `SELECT` per event:

```sql
GET DIAGNOSTICS v_log_rows = ROW_COUNT;
IF v_log_rows = 0 THEN
    PERFORM cairn_refuse_substitution(
        (SELECT content_address FROM event_log WHERE event_id = v_event_id),
        v_ca, v_event_id, 'submit_event');
END IF;
```

**One thing to verify rather than assume:** `PERFORM` overwrites `FOUND` and `ROW_COUNT`. Both doors
capture the INSERT outcome into a local (`v_log_rows` / `v_rows`) first — db/020's comment says why in
as many words — so the swap is safe *provided* nothing downstream reads `ROW_COUNT` or `FOUND`
expecting the INSERT's value. Walk both functions below the guard before editing.

In db/020 the guard's **position** is load-bearing and does not move: it sits above the
`cairn.remote_apply` marker clear and above the `cairn_project_late_custody` call, so a rival body
never reaches an applier (ADR-0070 decision 1, trap 10; pinned by
`late_custody_reaches_the_chart.rs::a_rival_body_never_reaches_an_applier`).

### 2.3 db/009 — once, after the `IF/ELSE`, and deliberately without `ROW_COUNT`

```sql
SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'restore_node_event');
```

Three properties, each chosen:

1. **No `GET DIAGNOSTICS`.** If the INSERT succeeded, `v_found = v_ca` and nothing raises. If it was a
   no-op over a differing row, it raises. If the row is somehow absent, `v_found` is NULL and
   `IS DISTINCT FROM` **refuses** — fail-closed, which is db/009's posture everywhere else.
2. **Robust to later edits.** A `ROW_COUNT`-based guard placed after the `IF/ELSE` is correct today
   only because the last statement executed in both branches is the INSERT. Someone later adding a
   statement inside either branch would disarm it **silently** — precisely the failure db/020's own
   comment warns about. Reading the row instead has no such coupling.
3. **Once, not twice.** Both branches insert into `node_event` under the same key, so one site covers
   both and there is no second copy to drift.

The cost is one extra `SELECT` per node event. The node plane is enrolls, peers, revokes and
supersedes — tens of events, not the 100k clinical path — so the §1.2 measurement (116.7 s against a
600 s budget, `crates/cairn-node/results/2026-09-10-macos-m3max.md`) is untouched. That is a claim the
plan should spot-check, not carry on assertion.

**A refusal aborts the whole restore, and that is correct.** `apply_medium`
(`crates/cairn-node/src/restore.rs:333-339`) propagates every door error with `?`. So one rival event
ends the ceremony. This is not a new posture: db/009 **already** aborts the whole restore on an unknown
node event type, an HLC wall past the drift ceiling, an author key resolving to no restored enroll, and
an over-ceiling event. A medium carrying two rival events under one `event_id` is a compromised or
corrupt medium, and the node plane is the **trust set** — restoring a node whose peer list was decided
by whoever appended last is a worse outcome than refusing and telling the operator to find another
copy. The alternative (refuse that record, continue, count it) needs node-plane completeness accounting
that does not exist, which is HANDOVER's framing of #615's second half and a slice of its own.

### 2.4 #614 — one aggregate query, one pure notice

`ClinicalRestoreReport` gains `deferred: usize`, filled **after** the apply loop by one query:

```sql
SELECT count(*) FROM event_deferred
```

**Why an aggregate and not a per-record probe.** A probe per record doubles the round-trips on the path
whose §1.2 budget was measured at 1.17 ms/event; one aggregate is O(1) and cannot move it.

**Why counting the whole table is right here, and the assumption to pin.** `restore` runs against an
un-enrolled database *before* `finalize_identity`, and nothing else writes to it in that window, so
every `event_deferred` row present came from this medium. That stays true on a **resumed** restore —
rows left by the earlier attempt are also this medium's. The assumption is narrow and should be written
down at the call site, because it is what makes a table-wide `count(*)` an honest answer to a question
about *this medium*.

**Why not change the door's return.** `apply_remote_event` RETURNS UUID, and the restore calls it
through `db.execute`, discarding it. Widening the return is a cross-crate signature change
`cairn-sync`'s `clinical_pull.rs` would have to follow — a per-crate test run would not catch the arity
gap, only a full-workspace `cargo test` would.

The notice is a **pure function** — `deferred_notice(n: usize) -> Option<String>` — so its wording is
testable with no database. It is printed inside the existing `counts.clinical > 0` block, and it names
`cairn-node deferred`, the subcommand (`main.rs:5229`) that exists for exactly this state:

```
clinical records: 412 applied, 0 already present, 0 refused (of 412 on the medium)
  · 7 of them carry an event type this build cannot classify. They ARE in the log
    and will project once this node is upgraded — no second restore is needed, and
    nothing is left on the medium. List them with `cairn-node deferred`.
```

**No sixth `Unrestored` field.** `Unrestored`'s doc states the set is *"closed at five"* and that a
sixth cause *"gets a deliberate decision, not a silent widening"* — decision 3 above is that decision,
and it is *no*. `the_cause_list_is_exactly_five` stays green, `is_complete()` is untouched, exit stays
0. This is consistent with ADR-0071's rule rather than an exception to it: the rule is a claim about
records reaching **the log**, and a deferred record has.

### 2.5 The published contract moves with the code

`restore --help`'s EXIT STATUS block currently names #614 as limit (b) — *"a record this build cannot
CLASSIFY is in the log and counted, but yields no chart until this node is upgraded"*. After this slice
that limit is **reported**, not merely disclosed, and the text must say so.

This is the fourth instance of the pattern PR #612 recorded three times: **a fix written under the
pressure of a finding is itself unreviewed code, and `--help` is part of the contract.** The wording is
hand-wrapped to 80 columns under `verbatim_doc_comment`, and it is asserted against the **spawned**
help, never the source text.

## 3. What this deliberately does not build

| Left open | Why |
|---|---|
| **#608's second half** — `cairn_project_late_custody`'s not-found arm returning silently | It inverts a live test (`heal_safe_dispatch.rs`) and changes refusal behaviour on a path ADR-0070 settled two days ago. #608 **narrows** to that one arm and stays open. |
| **#605** — an in-place `db/` function edit unguarded by #188 | No agreed mechanism exists; it is a code-plane design question. This slice is no longer *exposed* to it (52 → 53), which is not the same as fixing it. |
| **Node-plane completeness accounting** | #615's guard makes the substitution case loud, which is the reachable defect. A general "what did the node plane fail to apply" report is a separate slice. |
| **#613** — exit 0 having installed no custody key | A different question (recovery left short vs. records left behind) and an open maintainer decision. |

## 4. Testing

TDD throughout: every test below is written and seen to **fail for the stated reason** before the code
that makes it pass.

**#615 — DB-gated (`crates/cairn-node/tests/`)**

1. **The attack.** A medium carrying a genuine `peer.revoked`, plus a second validly-signed node event
   reusing that `node_event_id` with different content, positioned earlier in file order. Assert the
   restore refuses **by name** (`restore_node_event: … substitution refused`). Red before the guard:
   today it exits 0 having dropped the revocation. This is the test that must exist even if every other
   one is cut.
2. **Idempotence is untouched.** Re-restoring the *same* medium into the same database still passes —
   the guard must refuse a *rival*, never a repeat. The `ON CONFLICT DO NOTHING` idempotence that
   `apply_medium`'s doc promises is load-bearing on the resume path.
3. **Fail-closed on an absent row.** A direct call to `cairn_refuse_substitution(NULL, <ca>, …)` raises.
   This is the arm `<>` gets wrong and the reason the helper exists.
4. **The two existing guards still say exactly what they said.** Re-run the existing db/005 and db/020
   substitution tests unchanged, and assert the message text did not move.

**#614**

5. **Pure, no database.** `deferred_notice(0)` is `None`; `deferred_notice(n > 0)` names the count, says
   the records are in the log, says no second restore is needed, and names `cairn-node deferred`.
6. **DB-gated end to end.** A medium carrying an event of an unclassifiable type: the summary carries
   the line, the count is right, and the run still exits **0** with `Unrestored::default()`.

**Contract**

7. **The spawned `--help`** states the new exit-0 wording, and `--help` exits 0.

**Mutations.** Per the house pattern, each written down *with its expected outcome before the run*
(#594's lesson about reasoned vs. rationalised survivors), on a harness that refuses a dirty tree and
fails loudly if its own revert did not land. At minimum: `IS DISTINCT FROM` → `<>` (test 3 must kill);
delete the db/009 call (test 1 must kill); delete one of db/005's or db/020's calls (test 4 must kill);
`deferred` count hard-wired to 0 (test 6 must kill).

## 5. Risks

- **The guard makes a previously-succeeding restore fail.** By design, and only for a medium carrying
  rival content under one id — which no honest medium does. Pre-clinical posture: no deployed node can
  be holding such a medium today.
- **`PERFORM` clobbering `FOUND`/`ROW_COUNT` downstream in db/005 or db/020.** Verified by reading both
  functions below the guard, not assumed. If anything downstream *does* depend on either, the fallback
  is to re-capture it into a local immediately after the `PERFORM` — never to drop the helper and keep
  the inline copy, which is the outcome this slice exists to end.
- **A generation bump touches two loader lists.** `SCHEMA_GENERATION` is pinned by a test against the
  full `db/` list in `crates/cairn-node/src/db.rs`, and `cairn-sync` carries its own list. db/053 must
  land in **both** where the doors it guards are loaded, or a node started fresh at generation 53 misses
  the helper its doors call.
- **The docs build.** A new ADR needs its `mkdocs.yml` nav line **in the same commit**; the build runs
  `--strict` and a file absent from the nav aborts it. No local Rust gate sees this.

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the disaster-recovery ceremony — a clinic restoring its record from the backup
it keeps off-site, which on paper is carrying the box of charts back from storage and putting them on
the shelf.

**Steps:** paper *N* = 1 human act (fetch the box, put the charts back). Architecture-forced *M* = 3
(insert medium · run `restore` · supply the recovery code), unchanged by this slice — **it adds no
human act on the success path**. UI bundling target *K* = 3. `M > N` stands and is tracked as
[#512](https://github.com/cairn-ehr/cairn-ehr/issues/512); this slice neither widens nor narrows it.

**Time + cognitive load:** the measured budget is unchanged and not re-run — 100 003 events in
**116.7 s** against **600 s**, linear at 1.17 ms/event
(`crates/cairn-node/results/2026-09-10-macos-m3max.md`). The #614 count is one aggregate query, O(1);
the #615 read is one `SELECT` per **node**-plane event, of which a medium carries tens. Neither is on
the per-clinical-record path. **Cognitive load falls**: an operator who previously had to know that a
deferred record exists in order to look for it is now told, with the command that lists them.

The clinical-surface half of the ledger is unchanged — on paper, a chart that came back from storage in
a filing system nobody in the building can read is not a chart that came back, and saying so on the
restore's own last screen is the paper affordance (the box is visibly short) that the silent exit 0
removed.
