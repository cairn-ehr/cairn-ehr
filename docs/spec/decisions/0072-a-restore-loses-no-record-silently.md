# ADR-0072 — A restore loses no record silently

- **Status:** Accepted
- **Date:** 2026-09-17
- **Spec version at acceptance:** 0.74
- **Issues:** [#615](https://github.com/cairn-ehr/cairn-ehr/issues/615) ·
  [#614](https://github.com/cairn-ehr/cairn-ehr/issues/614) ·
  [#608](https://github.com/cairn-ehr/cairn-ehr/issues/608) (guard half)
- **Relates to:** [ADR-0071](0071-a-restore-that-left-records-behind-exits-incomplete.md) ·
  [ADR-0070](0070-a-late-key-reaches-the-chart.md) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) ·
  [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) ·
  [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
- **Amends:** ADR-0071's published exit-0 contract — not its rule, which is unchanged, but the
  list of limits the command states alongside it. Reverses nothing.

## Context

[ADR-0071](0071-a-restore-that-left-records-behind-exits-incomplete.md) published exit **0** as a
contract: *every record the medium carried is in this node's log*. Publishing a contract is what
turns every remaining way to reach it dishonestly into a defect worth naming — and three were found
within hours of that ADR being written, by asking the question its own review had just taught.

Two of them let `cairn-node restore` exit **0 having left a record behind**. They share nothing in
mechanism and everything in consequence, which is why they are decided together.

### #615 — the node plane discards a rival event in silence

Two of the three write doors refuse a **substitution** — a second, *different* event filed under an
`event_id` the log already holds. `submit_event` (`db/005`) and `apply_remote_event` (`db/020`) have
done so since their first review, because two nodes holding different bytes under one `event_id`
would diverge forever with no alarm.

The third, `restore_node_event` (`db/009`), did not. Its two
`INSERT … ON CONFLICT (node_event_id) DO NOTHING` sites carried no comparison at all, so the rival
was discarded without a word.

That door is **self-trusting** by design — any validly-signed `node.enrolled` is admitted without a
trust check, because a fresh node has no trust set to check against — and db/009's own comment
already establishes the reachability, in the course of justifying its clock-drift ceiling: *"the
medium can contain OTHER signers' events and is attacker-appendable."* So the attack is cheap.
Append an event carrying the `event_id` of the clinic's `peer.revoked`, positioned earlier in file
order. The genuine revocation is dropped. **The restored node comes back trusting a peer the clinic
had revoked**, and the summary reads `restored N event(s)` at exit 0.

The node plane is the **trust set**, so this is not a bookkeeping defect. The benign variants — a
UUID generator bug, a hand-merged medium — produce identical silence.

The count could never have caught it. `apply_medium` returns `Ok(events.len())`, which its own doc
calls *"the number of events PROCESSED … not the number newly inserted"*, so `events.len()` equals
the medium's node-plane count by construction. **The guard is the load-bearing half.**

### #614 — a clinical record is admitted uninterpreted and reported nowhere

Under [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md), `db/020` admits an event whose
`event_type` is absent from `event_type_class` *uninterpreted* — in its own words, it *"yields NO
projection rows and confers NO power"* — and returns `Ok`. Custody is orthogonal to classification,
so the restore's own custody check passed too, and `apply_clinical_plane` counted the record
`applied`. In the log, it genuinely is.

The summary then read `N applied, 0 already present, 0 refused (of N on the medium)`, every
`Unrestored` field was zero, and the run exited **0** — with those charts empty until somebody
noticed.

Per [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)'s additive schema
evolution, **a new clinical event type is the case that actually happens**; a whole new *plane* —
which ADR-0071 gives exit 3 — is rare. The realistic disaster-recovery box is a spare laptop one
release behind the live node, and it hits this case and not that one. Same underlying cause (the
medium was written by a newer Cairn), same remedy (upgrade), opposite report.

### #608 — and the reason the obvious fix for #615 was the wrong one

Both existing guards compared with `<>`:

```sql
IF (SELECT content_address FROM event_log WHERE event_id = v_event_id) <> v_ca THEN
```

The branch is reached only when the INSERT was a no-op, so the sub-select should always find a row.
If it ever does not, `<>` yields NULL, the `IF` does not fire, and **the guard passes silently**.
That state should be unreachable — under READ COMMITTED the next statement's snapshot sees the
committed conflicting row; under REPEATABLE READ or SERIALIZABLE the `ON CONFLICT DO NOTHING` raises
40001 first — so it is hardening rather than a live bug.

But it means the obvious fix for #615, *port db/005's guard into db/009*, would have written a known
fail-open into the safety-critical floor **a third time**, in the one door where the record at stake
is the node's own trust set. One invariant written twice had already become wrong in both places at
once; a third copy compounds that rather than repairing it.

## Decision

**1. One substitution refusal, shared by all three doors, compared with `IS DISTINCT FROM`.**
`cairn_refuse_substitution(p_found_ca, p_new_ca, p_event_id, p_door)` in a new
`db/053_substitution_guard.sql` raises when the two addresses differ **or when the stored one is
NULL**. A door that cannot establish what is stored under the id it is about to write refuses; on
the §9 safety-critical surface, *"should be unreachable"* is not a reason to pass.

The function is **pure — it reads no table**. Both addresses arrive as arguments, which is what lets
one function serve `event_log` and `node_event` without knowing about either, and is why each door
keeps its **own** read. What the doors share is the *decision*, not how each fetches the fact. The
door name is interpolated, so all three messages are the ones they already were.

**2. `restore_node_event` refuses a substitution, and the refusal aborts the ceremony.**
`apply_medium` propagates every door error, so one rival event ends the restore. This is not a new
posture: db/009 **already** aborts on an unknown node event type, an over-ceiling event, an HLC wall
past the drift ceiling, and an author key resolving to no restored enroll. A medium carrying two
rival events under one id is a compromised or corrupt medium, and restoring a node whose peer list
was decided by whoever appended last is a worse outcome than refusing and sending the operator to
find another copy.

Its guard sits **once, after the enroll/non-enroll branch, and deliberately without
`GET DIAGNOSTICS`.** A `ROW_COUNT` check there would be correct only because the last statement of
both branches happens to be the INSERT; someone later adding a statement inside either branch would
disarm it **silently** — the exact failure db/020's own comment warns about. Reading the row back
has no such coupling, fails closed on an absent row, and covers both branches from one site. It
costs one `SELECT` per **node-plane** event, of which a medium carries tens.

**3. A record this build cannot classify is REPORTED, and the exit status does not move.**
`ClinicalRestoreReport` gains a `deferred` count, filled by one aggregate query after the apply loop,
and the summary names it alongside the command that lists them (`cairn-node deferred`).

It is **not** a sixth [`Unrestored`](0071-a-restore-that-left-records-behind-exits-incomplete.md)
cause. `Unrestored`'s doc states the set is *"closed at five"* and that a sixth *"gets a deliberate
decision, not a silent widening"*; this is that decision, and it is **no**. The reasoning is
ADR-0071's own rule taken at its word: exit 0 claims the records reached **the log**, and a deferred
record has. `connect_and_load_schema` re-adjudicates deferred events, so an upgrade heals this with
**nothing left on the medium and no second restore**. Reporting it as INCOMPLETE would call the most
cheaply-repaired outcome in the whole vocabulary a failed recovery.

**The defect was the silence, and only the silence.**

**4. The published contract moves with the code.** `restore --help`'s EXIT STATUS block listed #614
as a known *limit* of exit 0. Once the summary reports those records that sentence is false, and a
cron-wrapper author reading it is told the command is silent about something it now names. The block
is amended in the same change, and pinned by a test against the **spawned** help.

## Why decisions 2 and 3 point in opposite directions

They are answers to different questions, and the difference is what the medium still holds.

A substituted node event is a record this node **cannot have** — the bytes are on the medium under
an id already taken, and no later command reaches them. It is also, uniquely among these cases,
*evidence that the medium is not what it claims to be*. A deferred clinical event is a record this
node **already has** and cannot yet *read*; the medium is complete, this build is not, and the
remedy touches the binary rather than the backup.

ADR-0071's vocabulary is organised around exactly that axis — *did a record fail to reach the log* —
so the two cases land on opposite sides of it without any special pleading.

## Alternatives rejected

- **Port db/005's guard verbatim into db/009.** The smallest diff, and it writes #608's fail-open
  into the floor a third time. Rejected in the brainstorm before any code was written.
- **Fix #608 outright**, including `cairn_project_late_custody`'s silent not-found arm. That inverts
  a live test in `heal_safe_dispatch.rs` and changes refusal behaviour on a path ADR-0070 settled two
  days earlier. #608 **narrows** to that one arm and stays open.
- **Put the helper in `db/001`, beside `cairn_decode_hex_or_raise`.** Its closest sibling lives
  there, and it needs no new file — but an in-place edit carries no `SCHEMA_GENERATION` bump, so all
  three doors would stay replaceable by an older binary in silence
  ([#605](https://github.com/cairn-ehr/cairn-ehr/issues/605)). 52 → 53 buys the #188 downgrade guard.
- **Refuse the rival node event but continue the restore, counting it.** Symmetric with the clinical
  plane's skip-and-pen. It needs node-plane completeness accounting that does not exist, and it would
  restore a node whose trust set is known to be contested while reporting success.
- **Give a deferred record exit 3**, symmetric with the unroutable-plane case. Rejected under
  decision 3: it reports as INCOMPLETE a state that an upgrade heals without touching the medium.
- **Widen what exit 0 claims** — drop "and usable", so deferred records stop falsifying it. Tidier,
  but it weakens the promise a cron wrapper reads, and #613 would need the same treatment.

## Consequences

- A fourth door needing this guard **calls** `cairn_refuse_substitution`. A fourth inline copy is
  the #608 shape returning, and `substitution_guard_is_single_source.rs` fails on one.
- A restore over a medium carrying rival content under one id now **fails** where it used to
  succeed. No honest medium does this, and the project is pre-clinical, so no deployed node can be
  holding one.
- `SCHEMA_GENERATION` is **53**. `db/053` is in **both** loader lists: `cairn-sync`'s subset
  legitimately lags `db/` for node-only migrations, but it carries db/005 and db/020, which call the
  helper — and PL/pgSQL binds function names at *execution*, so omitting it would load cleanly and
  fail at the first write.
- Exit 0 now has **one** stated limit rather than two.

## Residuals — named, not assumed away

- **[#608](https://github.com/cairn-ehr/cairn-ehr/issues/608)** — `cairn_project_late_custody`'s
  not-found arm still returns silently. Narrowed, not closed.
- **[#605](https://github.com/cairn-ehr/cairn-ehr/issues/605)** — an in-place `db/` function edit is
  still unprotected in general. This change is no longer *exposed* to it, which is not the same as
  fixing it.
- **[#613](https://github.com/cairn-ehr/cairn-ehr/issues/613)** — a restore can still exit 0 having
  installed no custody key, when the medium carried no clinical records. A different question
  (*recovery left short* rather than *records left behind*) and an open decision.
- **Node-plane completeness accounting** does not exist. Decision 2 makes the substitution case
  loud, which is the reachable defect; a general *"what did the node plane fail to apply"* report is
  a separate slice.
- **`deferred_count` counts the whole table.** Honest because a restore runs on an un-enrolled
  database before `finalize_identity` and nothing else writes in that window — so every
  `event_deferred` row present came from this medium, resumed runs included. A future caller running
  it against a database with other history would get a number that no longer means what it says.
- **[#603](https://github.com/cairn-ehr/cairn-ehr/issues/603) /
  [#604](https://github.com/cairn-ehr/cairn-ehr/issues/604)** — the cross-transaction races ADR-0070
  named are untouched here.
